//! Terminal Markdown rendering for the manifest `description` field.
//!
//! [`render_markdown_to_stderr`] shells out to `glow` (charmbracelet/glow)
//! for full ANSI rendering; [`first_line_plain`] extracts a plain-text title line
//! without invoking glow, for row-oriented output like `flox envs` where a
//! multi-line ANSI render would break the table.

use std::io::Write;
use std::os::fd::AsFd;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::{env, thread};

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::utils::message::stderr_supports_color;

static GLOW_BIN: LazyLock<String> =
    LazyLock::new(|| env::var("GLOW_BIN").unwrap_or(env!("GLOW_BIN").to_string()));

/// Render `input` (Markdown) at `width` columns by shelling out to
/// `glow`, which writes straight to this process's stderr.
///
/// stderr, not stdout: every other message `flox activate` prints goes
/// through `tracing`, whose subscriber writes there, so rendering to
/// stdout would put the description on a different stream from the text
/// around it — visible the moment anyone redirects one and not the other.
///
/// Returns `Err` with a short reason, having written nothing, if `glow`
/// is missing, unspawnable, or exits non-zero — a broken or absent
/// renderer must never break activation. Glow validates its arguments
/// before emitting any output, so a failure cannot leave a partial
/// render on the terminal.
pub fn render_markdown_to_stderr(input: &str, width: usize) -> Result<(), String> {
    // `pink` is the only glamour style (of `pink`, `dark`, `light`, `auto`,
    // `dracula`, `tokyo-night`) that renders an ATX/Setext heading marker as
    // a glyph (`▌` for h2, `┃` for h3) instead of leaving a literal `## `;
    // verified directly against this repo's pinned glow (2.1.1). `notty`
    // additionally flattens nested list indentation
    // (charmbracelet/glamour#184) — that is glow's behavior for a
    // non-color terminal, accepted as-is rather than patched around here.
    // A `description = """` block is usually indented to sit with the rest
    // of the manifest, and TOML keeps that whitespace. Four spaces is an
    // indented code block in CommonMark, so without this a description
    // indented that far renders as literal text rather than Markdown.
    // `dedent` takes only the common prefix, leaving relative indentation
    // that means something -- nested items, fenced contents -- intact.
    let input = &textwrap::dedent(input);

    let style = if stderr_supports_color() {
        "pink"
    } else {
        "notty"
    };

    render_with_glow(input, width, style)
}

/// Spawn `glow` with `input` on stdin, letting it render straight to this
/// process's stderr. Returns `Err` with a short reason if it could not be
/// spawned or exited non-zero, in which case nothing was written.
fn render_with_glow(input: &str, width: usize, style: &str) -> Result<(), String> {
    // Hand glow a dup of our own stderr as its stdout, rather than
    // `Stdio::inherit()` (which would give it stdout) or a pipe we then
    // relay. Nothing is buffered, so there is no output to validate as
    // UTF-8 and no second thread needed to drain a pipe.
    let stderr_fd = std::io::stderr()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|e| format!("could not duplicate stderr: {e}"))?;

    let mut child = Command::new(&*GLOW_BIN)
        .args(["-w", &width.to_string(), "-s", style, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stderr_fd))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run 'glow': {e}"))?;

    // Write stdin on another thread: glow buffers all of it before
    // producing output, so writing inline could deadlock against a full
    // pipe. Dropping the handle closes glow's end and gives it EOF —
    // without which glow blocks even when given a file argument.
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "could not write to 'glow'".to_string())?;
    let owned_input = input.to_string();
    let writer = thread::spawn(move || {
        let _ = stdin.write_all(owned_input.as_bytes());
    });

    let output = child
        .wait_with_output()
        .map_err(|e| format!("could not run 'glow': {e}"));
    let _ = writer.join();
    let output = output?;

    if output.status.success() {
        return Ok(());
    }

    // glow only ever complains about the arguments *we* passed — a bad
    // style or width — never about the user's Markdown, which it renders
    // as best it can. Carry its own words anyway: a packaging or version
    // problem is far easier to report with them than without.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    Err(if detail.is_empty() {
        format!("'glow' exited with {}", output.status)
    } else {
        detail.to_string()
    })
}

/// Extract the first line of `input` as plain text, with Markdown syntax
/// stripped.
///
/// A leading heading — ATX (`# Title`) or Setext (`Title` over `===`) —
/// yields its title text with the marker/underline dropped; pulldown-cmark
/// parses both forms to the same heading event, so no separate case is
/// needed for either, and a heading wrapped over several source lines
/// yields all of its text: it is one title. A leading paragraph instead
/// yields only its first line -- the walk stops at the first
/// `SoftBreak`/`HardBreak` -- similarly stripped of inline syntax (emphasis, code spans, link brackets) via the
/// same mechanism: those characters are parsed into structural tags rather
/// than emitted as text, so concatenating only `Text`/`Code` events already
/// excludes them. Any other leading construct (list, code block, quote)
/// falls back to the raw first line, since a document opening with one of
/// those has no title-like fragment for these to clean up.
pub fn first_line_plain(input: &str) -> String {
    let input = &textwrap::dedent(input);
    let mut parser = Parser::new(input);

    let Some(first_event) = parser.next() else {
        return String::new();
    };

    // A line break means different things in the two blocks that can open
    // a description. A heading has already declared itself a title, so a
    // Setext H1 wrapped over two source lines is one title and all of it
    // belongs in the row. A paragraph has declared nothing, so its break
    // is the author's own division of prose -- a better place to stop
    // than the arbitrary column truncation would pick downstream.
    let opens_heading = match first_event {
        Event::Start(Tag::Heading { .. }) => true,
        Event::Start(Tag::Paragraph) => false,
        _ => return input.lines().next().unwrap_or("").trim().to_string(),
    };

    let mut text = String::new();
    for event in parser {
        match event {
            Event::End(TagEnd::Heading(_)) | Event::End(TagEnd::Paragraph) => break,
            Event::SoftBreak | Event::HardBreak => {
                if opens_heading {
                    text.push(' ');
                } else {
                    break;
                }
            },
            Event::Text(t) | Event::Code(t) => text.push_str(&t),
            _ => {},
        }
    }

    text.trim().to_string()
}

/// Truncate `s` to fit within `width` terminal columns, appending an
/// ellipsis when truncated.
///
/// Width-aware per Unicode East Asian Width (a wide CJK character counts as
/// 2 columns, a combining mark counts as 0) so a `flox envs` row never
/// overflows regardless of script. Never panics: `width == 0` and content
/// that doesn't fit even the ellipsis both degrade to an empty string
/// rather than indexing past a budget that went negative.
pub fn truncate_to_width(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }

    const ELLIPSIS: char = '…';
    let ellipsis_width = ELLIPSIS.width().unwrap_or(1);
    if width < ellipsis_width {
        return String::new();
    }

    let budget = width - ellipsis_width;
    let mut truncated = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if used + ch_width > budget {
            break;
        }
        truncated.push(ch);
        used += ch_width;
    }
    truncated.push(ELLIPSIS);

    truncated
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn render_with_glow_accepts_the_arguments_this_module_passes() {
        // Check both styles against the bundled renderer so an incompatible
        // flag or style fails here instead of during activation.
        for style in ["pink", "notty"] {
            assert!(
                render_with_glow("# Title\n\nBody.\n", 80, style).is_ok(),
                "glow rejected the arguments for style {style}"
            );
        }
    }

    #[test]
    fn first_line_plain_atx_h1_strips_marker() {
        assert_eq!(first_line_plain("# My Title\n\nBody."), "My Title");
    }

    #[test]
    fn first_line_plain_setext_h1_strips_underline() {
        assert_eq!(first_line_plain("My Title\n========\n\nBody."), "My Title");
    }

    #[test]
    fn first_line_plain_strips_inline_syntax() {
        assert_eq!(
            first_line_plain("A **bold** and `code` and [link](url)."),
            "A bold and code and link."
        );
    }

    #[test]
    fn first_line_plain_sees_through_manifest_indentation() {
        // Four spaces is an indented code block in CommonMark, so without
        // dedenting this yields the literal "# Indented Title".
        assert_eq!(
            first_line_plain("    # Indented Title\n\n    Body.\n"),
            "Indented Title"
        );
    }

    #[test]
    fn first_line_plain_keeps_a_setext_heading_wrapped_over_two_lines() {
        // One H1 whose title happens to wrap. Stopping at the break would
        // drop half a title, and a Setext opener is the convention the
        // design recommends.
        assert_eq!(
            first_line_plain("My Environment\nFor Testing\n==============\n"),
            "My Environment For Testing"
        );
    }

    #[test]
    fn first_line_plain_keeps_an_atx_heading_continued_onto_a_second_line() {
        // An ATX heading ends at its own newline, so a following line is a
        // separate paragraph and must not join the title.
        assert_eq!(
            first_line_plain("# My Environment\nnot part of the title\n"),
            "My Environment"
        );
    }

    #[test]
    fn first_line_plain_stops_at_a_hard_wrapped_line_break() {
        assert_eq!(
            first_line_plain("A title that\nwraps onto two lines"),
            "A title that"
        );
    }

    #[test]
    fn first_line_plain_atx_h2_strips_marker() {
        assert_eq!(first_line_plain("## Subtitle\n"), "Subtitle");
    }

    #[test]
    fn first_line_plain_leading_list_falls_back_to_raw_line() {
        assert_eq!(first_line_plain("- one\n- two\n"), "- one");
    }

    #[test]
    fn first_line_plain_empty_input() {
        assert_eq!(first_line_plain(""), "");
    }

    #[test]
    fn truncate_to_width_no_truncation_needed() {
        assert_eq!(truncate_to_width("short", 80), "short");
    }

    #[test]
    fn truncate_to_width_ascii_truncates_with_ellipsis() {
        assert_eq!(truncate_to_width("abcdefghij", 5), "abcd…");
    }

    #[test]
    fn truncate_to_width_zero_width_never_panics() {
        assert_eq!(truncate_to_width("hello", 0), "");
    }

    #[test]
    fn truncate_to_width_wide_cjk_never_panics() {
        // Each character is 2 columns wide; budget of 5 fits 2 of them
        // (4 columns) plus a 1-column ellipsis.
        let result = truncate_to_width("你好世界", 5);
        assert_eq!(result, "你好…");
    }

    #[test]
    fn truncate_to_width_combining_marks_never_panic() {
        // "e" + combining acute accent (U+0301): 0-width combining mark
        // must not desync the byte/column budget.
        let input = "cafe\u{0301} au lait";
        let result = truncate_to_width(input, 6);
        assert!(result.width() <= 6);
    }

    #[test]
    fn truncate_to_width_emoji_at_boundary_never_panics() {
        // Emoji are wide (2 columns); a budget that lands mid-emoji must
        // drop the whole character rather than panic on a partial width.
        let result = truncate_to_width("hi 🎉🎉🎉", 5);
        assert!(result.width() <= 5);
    }
}
