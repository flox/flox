use std::io::{BufWriter, Write};
use std::num::NonZeroU64;
use std::process::ExitCode;

use anstream::adapter::StripBytes;
use anyhow::Result;
use bpaf::Bpaf;
use floxhub_client::{FactoryClientError, FactoryClientTrait};
use futures::StreamExt;
use tracing::instrument;

use crate::utils::message;
use crate::{Exit, subcommand_metric};

/// Print the logs for a single Flox Factory build.
#[derive(Debug, Clone, PartialEq, Bpaf)]
pub struct Logs {
    /// Print the raw log bytes without sanitizing terminal escape sequences
    #[bpaf(long)]
    pub raw: bool,

    /// Build ID to fetch logs for
    #[bpaf(positional("ID"))]
    pub id: NonZeroU64,
}

impl Logs {
    #[instrument(name = "logs", skip_all)]
    pub async fn handle(self, client: &impl FactoryClientTrait) -> Result<()> {
        subcommand_metric!("factory::logs");

        let mut stream = match client.get_build_logs(self.id.get() as i64).await {
            Ok(stream) => stream,
            Err(FactoryClientError::NotFound) => {
                message::error(format!("No logs available for build {}.", self.id));
                return Err(Exit(ExitCode::from(2)).into());
            },
            Err(other) => return Err(super::user_facing_error(other, None)),
        };

        // stdout is always `LineWriter`-backed, so it flushes on every newline.
        // Wrap it in a BufWriter to batch the log into ~8KB writes and spare
        // that per-line flush — this is a one-shot dump, not a live tail.
        // `finish` does the single explicit flush at the end, which also
        // surfaces a write error that BufWriter's drop-flush would swallow.
        let stdout = std::io::stdout();
        let mut writer = LogWriter::new(BufWriter::new(stdout.lock()), self.raw);

        while let Some(chunk) = stream.next().await {
            writer.write_chunk(chunk?.as_ref())?;
        }
        writer.finish()?;

        Ok(())
    }
}

/// Streams log byte-chunks to an output, either:
///
/// - sanitized (default): strips all terminal escape sequences, including
///   colour, to prevent injection: https://cwe.mitre.org/data/definitions/150.html
/// - raw: prints verbatim, byte-for-byte.
///
/// `StripBytes` holds the parser's cross-chunk state, so a sequence or
/// multibyte UTF-8 character that straddles a chunk boundary is handled.
struct LogWriter<W: Write> {
    writer: W,
    /// `None` selects raw, byte-for-byte output.
    stripper: Option<StripBytes>,
}

impl<W: Write> LogWriter<W> {
    fn new(writer: W, raw: bool) -> Self {
        Self {
            writer,
            stripper: (!raw).then(StripBytes::new),
        }
    }

    /// Write one chunk: sanitized text, or the raw bytes unchanged.
    fn write_chunk(&mut self, chunk: &[u8]) -> std::io::Result<()> {
        let Some(stripper) = &mut self.stripper else {
            return self.writer.write_all(chunk);
        };
        for printable in stripper.strip_next(chunk) {
            self.writer.write_all(printable)?;
        }
        Ok(())
    }

    /// Flush the underlying buffer.
    fn finish(mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use pretty_assertions::assert_eq;
    use tracing::instrument::WithSubscriber;

    use super::*;
    use crate::Exit;
    use crate::commands::factory::test_helpers::StubFactoryClient;

    /// Render chunks through a `LogWriter`, returning the bytes written.
    fn render(chunks: &[&[u8]], raw: bool) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        let mut writer = LogWriter::new(&mut out, raw);
        for chunk in chunks {
            writer.write_chunk(chunk).unwrap();
        }
        writer.finish().unwrap();
        out
    }

    #[test]
    fn raw_writes_bytes_byte_for_byte() {
        // The escape sequences must survive unchanged under --raw.
        let input: &[u8] = b"\x1b[31mred\x1b[0m\x1b]0;title\x07\x9bdata\n";
        assert_eq!(render(&[input], true), input);
    }

    #[test]
    fn sanitized_strips_ansi_escapes() {
        // A representative mix: SGR colour, an OSC title terminated by BEL, and
        // an OSC hyperlink terminated by ST. All escapes go; text stays.
        let out = render(
            &[b"\x1b[31mred\x1b[0m \x1b]0;title\x07tail \x1b]8;;http://x\x1b\\link\n"],
            false,
        );
        assert_eq!(out, b"red tail link\n");
    }

    #[test]
    fn sanitized_strips_escape_split_across_chunk_boundary() {
        // The CSI sequence is split mid-escape across the two chunks; one
        // `StripBytes` carried across both must still strip it.
        let out = render(&[b"foo\x1b[3", b"1mbar\n"], false);
        assert_eq!(out, b"foobar\n");
    }

    #[test]
    fn sanitized_keeps_text_tabs_newlines_and_carriage_returns() {
        // Whitespace controls are structural in logs and must survive.
        let input: &[u8] = b"line1\r\n\tline2";
        assert_eq!(render(&[input], false), input);
    }

    #[test]
    fn sanitized_strips_esc_and_all_c1_control_bytes() {
        // The security guarantee: no byte that can drive the terminal may reach
        // it. Feed each byte value on its own — so the outcome depends only on
        // that byte, not on neighbouring stream state — and confirm that ESC
        // (0x1b) and every 8-bit C1 control (0x80..=0x9f, which includes the
        // CSI 0x9b and OSC 0x9d introducers) is stripped. The loop also exercises
        // sanitizing over every byte value without panicking. Surviving bytes
        // are ordinary text (e.g. UTF-8 lead bytes) that cannot introduce an
        // escape sequence.
        for byte in 0u8..=255 {
            let out = render(&[&[byte]], false);
            if byte == 0x1b || (0x80..=0x9f).contains(&byte) {
                assert!(
                    !out.contains(&byte),
                    "escape control byte {byte:#04x} survived sanitizing: {out:?}"
                );
            }
        }
    }

    #[test]
    fn sanitized_leaves_c1_sequence_payload_as_text() {
        // A sequence introduced by a raw 8-bit C1 byte has its introducer (and
        // any terminator) removed so it cannot drive the terminal, but the
        // payload bytes are not escapes and remain as inert visible text.
        //
        // CSI 0x9b loses only the introducer.
        assert_eq!(render(&[b"\x9b31mRED\x9b0m"], false), b"31mRED0m");
        // OSC 0x9d loses the introducer and its BEL terminator.
        assert_eq!(render(&[b"\x9d0;title\x07tail"], false), b"0;titletail");
    }

    #[tokio::test]
    async fn not_found_renders_message_and_returns_exit_error() {
        let client = StubFactoryClient::with_not_found();
        let args = Logs {
            raw: false,
            id: NonZeroU64::new(42).unwrap(),
        };

        let (subscriber, writer) = test_subscriber_message_only();
        let err = async { args.handle(&client).await.unwrap_err() }
            .with_subscriber(subscriber)
            .await;

        assert!(err.is::<Exit>(), "expected an Exit error, got {err:?}");
        assert_eq!(
            writer.to_string(),
            "✘ ERROR: No logs available for build 42.\n"
        );
    }
}
