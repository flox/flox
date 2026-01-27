pub mod errors;
pub mod gomap;
pub mod guard;
pub mod logging;

use std::collections::HashSet;
use std::fmt::{Display, Write};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::thread::{self, JoinHandle};
use std::time::SystemTime;
use std::{env, fs, io};

pub use flox_core::traceable_path;
use serde::Serialize;
use thiserror::Error;
use tracing::{debug, trace};
use walkdir;

use self::errors::IoError;

/// Whether the CLI is being run in CI
/// We could probably be more thorough about what we're checking,
/// but for now just use the `CI` environment variable
pub static IN_CI: LazyLock<bool> = LazyLock::new(|| env::var("CI").is_ok());

/// Whether the CLI is being run in a flox containerd context
pub static IN_CONTAINERD: LazyLock<bool> = LazyLock::new(|| env::var("FLOX_CONTAINERD").is_ok());

pub static FLOX_INTERPRETER: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from(env::var("FLOX_INTERPRETER").unwrap_or(env!("FLOX_INTERPRETER").to_string()))
});

/// Heuristics table for inferring invocation sources from environment
const INFERENCE_HEURISTICS: &[(&str, &str)] = &[
    // CI environments
    ("GITHUB_ACTIONS", "ci.github-actions"),
    ("GITLAB_CI", "ci.gitlab"),
    ("CIRCLECI", "ci.circleci"),
    ("JENKINS_HOME", "ci.jenkins"),
    ("BUILDKITE", "ci.buildkite"),
    ("TRAVIS", "ci.travis"),
    // Agentic tooling
    ("ANTHROPIC_BEDROCK_AWS_REGION", "agentic.unknown"),
    ("LANGCHAIN_API_KEY", "agentic.unknown"),
    ("OPENAI_API_KEY", "agentic.unknown"),
];

/// Detect invocation sources from environment heuristics
fn detect_heuristics() -> impl Iterator<Item = String> {
    INFERENCE_HEURISTICS
        .iter()
        .filter_map(|(env_var, source)| env::var(env_var).ok().map(|_| source.to_string()))
}

/// Detect all invocation sources for the current CLI invocation
///
/// Returns a deduplicated vector of invocation source identifiers.
/// Sources are detected from:
/// 1. Explicit FLOX_INVOCATION_SOURCE environment variable (comma-separated)
/// 2. CI environment (CI=true or specific CI platform env vars)
/// 3. Containerd context (FLOX_CONTAINERD env var)
/// 4. Inference heuristics for agentic tooling and other contexts
pub fn detect_invocation_sources() -> Vec<String> {
    let mut sources = HashSet::new();

    // Explicit sources from FLOX_INVOCATION_SOURCE
    if let Ok(explicit) = env::var("FLOX_INVOCATION_SOURCE") {
        for source in explicit.split(',').map(str::trim) {
            if !source.is_empty() {
                sources.insert(source.to_string());
            }
        }
    }

    // CI detection (generic)
    if *IN_CI {
        sources.insert("ci".to_string());
    }

    // Containerd detection (backward compatibility)
    if *IN_CONTAINERD {
        sources.insert("containerd".to_string());
    }

    // Apply inference heuristics
    sources.extend(detect_heuristics());

    // Convert to sorted vec for consistent ordering
    let mut result: Vec<String> = sources.into_iter().collect();
    result.sort();
    result
}

/// Detected invocation sources for this CLI run, computed once at startup
pub static INVOCATION_SOURCES: LazyLock<Vec<String>> = LazyLock::new(detect_invocation_sources);

#[derive(Error, Debug)]
pub enum FindAndReplaceError {
    #[error("walkdir error: {0}")]
    WalkDir(walkdir::Error),
    #[error("Error opening template file")]
    OpenTemplateFile(std::io::Error),
    #[error("Error reading template file contents")]
    ReadTemplateFile(std::io::Error),
    #[error("Error writing to template file")]
    WriteTemplateFile(std::io::Error),
}

/// Using fs::copy copies permissions from the Nix store, which we don't want, so open (or
/// create) the files and copy with io::copy
pub fn copy_file_without_permissions(
    from: impl AsRef<Path>,
    to: impl AsRef<Path>,
) -> Result<(), IoError> {
    let mut to_file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(&to)
        .map_err(|io_err| IoError::Open {
            file: to.as_ref().to_path_buf(),
            err: io_err,
        })?;
    let mut from_file = fs::File::open(&from).map_err(|io_err| IoError::Open {
        file: from.as_ref().to_path_buf(),
        err: io_err,
    })?;

    io::copy(&mut from_file, &mut to_file).map_err(|io_err| IoError::Copy {
        file: from.as_ref().to_path_buf(),
        err: io_err,
    })?;
    Ok(())
}

/// Get the mtime of a file, directory or symlink
///
/// Unlike `std::fs::metadata`, this function will not follow symlinks,
/// but return the mtime of the symlink itself.
///
/// If the file or directory does not exist,
/// or if the mtime cannot be determined, return [SystemTime::UNIX_EPOCH]
pub fn mtime_of(path: impl AsRef<Path>) -> SystemTime {
    let path = path.as_ref();
    'time: {
        let metadata = if path.is_symlink() {
            let Ok(metadata) = fs::symlink_metadata(path) else {
                debug!("Could not get metadata for {path:?} using default time");
                break 'time SystemTime::UNIX_EPOCH;
            };
            metadata
        } else {
            let Ok(metadata) = path.metadata() else {
                debug!("Could not get metadata for {path:?} using default time");
                break 'time SystemTime::UNIX_EPOCH;
            };
            metadata
        };
        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH)
    }
}

/// An extension trait for [std::process::Command]
pub trait CommandExt {
    /// Provide a [DisplayCommand] that can be used to display
    /// POSIX like formatting of the command.
    fn display(&self) -> DisplayCommand<'_>;
}

impl CommandExt for std::process::Command {
    fn display(&self) -> DisplayCommand<'_> {
        DisplayCommand(self)
    }
}

pub struct DisplayCommand<'a>(&'a std::process::Command);

impl Display for DisplayCommand<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let command = self.0;

        let args = command
            .get_args()
            .map(|a| shell_escape::escape(a.to_string_lossy()));

        let mut envs = command
            .get_envs()
            .map(|(k, v)| {
                let Some(v) = v else {
                    return format!("-u {}", k.to_string_lossy());
                };

                format!(
                    "{k}={v}",
                    k = k.to_string_lossy(),
                    v = shell_escape::escape(v.to_string_lossy())
                )
            })
            .peekable();

        if envs.peek().is_some() {
            write!(f, "env ")?;
            for env in envs {
                write!(f, "{} ", env)?;
            }
        }

        write!(f, "{}", command.get_program().to_string_lossy())?;
        for arg in args {
            write!(f, " {}", arg)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
/// Allow synchronous processing of reader output
pub struct WireTap<Context> {
    reader_handle: JoinHandle<Context>,
}

impl WireTap<()> {
    /// Create a new [WireTap] that will read lines from the `reader`.
    /// The `tap_fn` is called with each line read from the reader and a mutable reference to the provided context.
    /// The context can be used to store state between calls to the `tap_fn`.
    ///
    /// This function is mainly used for testing, where the context is used to store a [tracing::Dispatch].
    /// Use [WireTap::tap_lines] for a simpler version that does not require a context.
    pub(crate) fn tap_lines_with_context<Reader, TapContext, TapFn>(
        reader: Reader,
        mut context: TapContext,
        tap_fn: TapFn,
    ) -> WireTap<TapContext>
    where
        Reader: Read + Send + 'static,
        TapContext: Send + 'static,
        TapFn: Fn(&mut TapContext, &str) + Send + 'static,
    {
        let handle = thread::spawn(move || {
            let mut s = BufReader::new(reader);
            let mut line_buf = Vec::new();
            loop {
                match s.read_until(b'\n', &mut line_buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&line_buf).into_owned();
                        tap_fn(&mut context, line.trim_end_matches("\n"));

                        line_buf.clear();
                    },
                    Err(err) => {
                        trace!("Error reading line: {err}");
                        continue;
                    },
                }
            }
            context
        });

        WireTap {
            reader_handle: handle,
        }
    }
}
impl WireTap<String> {
    /// Create a new [WireTap] that will read lines from the reader
    /// and call the provided function with each line.
    /// The output will be collected into a [String].
    pub fn tap_lines<R, F>(r: R, tap_fn: F) -> WireTap<String>
    where
        R: Read + Send + 'static,
        F: Fn(&str) + Send + 'static,
    {
        WireTap::tap_lines_with_context(r, String::new(), move |buf, line| {
            writeln!(buf, "{line}").expect("Error writing line");
            tap_fn(line)
        })
    }
}
impl<Buffer> WireTap<Buffer> {
    /// Wait for the reader thread to finish and return the buffer
    /// containing the collected output.
    /// This will block until the reader thread finishes.
    /// If the reader thread panics, this will also panic.
    pub fn wait(self) -> Buffer {
        self.reader_handle.join().expect("Reader thread panicked")
    }
}

/// An extension trait for [std::io::Read] that allows creating [WireTap]s,
/// from a reader that will collect the output into a buffer
/// while allowing synchronous processing of the output.
pub trait ReaderExt
where
    Self: Read + Send + Sized + 'static,
{
    fn tap_lines<F>(self, tap_fn: F) -> WireTap<String>
    where
        F: Fn(&str) + Send + 'static;
}

impl<R> ReaderExt for R
where
    Self: Read + Send + Sized + 'static,
{
    fn tap_lines<F>(self, tap_fn: F) -> WireTap<String>
    where
        F: Fn(&str) + Send + 'static,
    {
        WireTap::tap_lines(self, tap_fn)
    }
}

/// Call serde_json::to_string_pretty and append a newline
pub fn serialize_json_with_newline<T>(value: &T) -> Result<String, serde_json::Error>
where
    T: ?Sized + Serialize,
{
    let mut serialized = serde_json::to_string_pretty(value)?;
    serialized.push('\n');
    Ok(serialized)
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use logging::test_helpers::test_subscriber;
    use pretty_assertions::assert_eq;
    use tracing::error;

    use super::*;

    #[test]
    fn tap_reader() {
        let (subscriber, writer) = test_subscriber();
        let dispatcher = tracing::Dispatch::new(subscriber);
        let content = indoc! {
            "
            Bytes pile high like snow
            Reader parses line by line
            Winter of data

            -- Phind
            "
        };

        // test that the `WireTap::tap_lines` collects the output
        let collected = WireTap::tap_lines(content.as_bytes(), |_| {}).wait();
        assert_eq!(collected, content);

        // Test that the lines can be logged
        // using the `WireTap::tap_lines_into_with_context` helper.
        // [tracing::dispatcher::with_default] only allows to set the dispatcher for the current thread,
        // so we need to use the `WireTap::tap_lines_into_with_context` helper to pass the dispatcher to the reader thread as context.
        // Unfortunately, that means that we basically only have a complex way of testing
        // that the tap function is called with the correct lines.
        // The behavior of a global dispatcher can not be tested with unit tests here.
        WireTap::tap_lines_with_context(content.as_bytes(), dispatcher, |dispatcher, line| {
            tracing::dispatcher::with_default(dispatcher, || error!("{line}"))
        })
        .wait();
        let logged = writer.to_string();

        assert_eq!(logged, content);
    }

    #[test]
    fn test_detect_invocation_sources_explicit() {
        temp_env::with_var("FLOX_INVOCATION_SOURCE", Some("vscode.terminal"), || {
            let sources = detect_invocation_sources();
            assert!(sources.contains(&"vscode.terminal".to_string()));
        });
    }

    #[test]
    fn test_detect_invocation_sources_multiple_explicit() {
        temp_env::with_var(
            "FLOX_INVOCATION_SOURCE",
            Some("ci.github-actions,agentic.flox-mcp"),
            || {
                let sources = detect_invocation_sources();
                assert!(sources.contains(&"ci.github-actions".to_string()));
                assert!(sources.contains(&"agentic.flox-mcp".to_string()));
            },
        );
    }

    #[test]
    fn test_detect_invocation_sources_ci() {
        temp_env::with_var("CI", Some("true"), || {
            let sources = detect_invocation_sources();
            assert!(sources.contains(&"ci".to_string()));
        });
    }

    #[test]
    fn test_detect_invocation_sources_github_actions() {
        temp_env::with_var("GITHUB_ACTIONS", Some("true"), || {
            let sources = detect_invocation_sources();
            assert!(sources.contains(&"ci.github-actions".to_string()));
        });
    }

    #[test]
    fn test_detect_invocation_sources_containerd() {
        temp_env::with_var("FLOX_CONTAINERD", Some("1"), || {
            let sources = detect_invocation_sources();
            assert!(sources.contains(&"containerd".to_string()));
        });
    }

    #[test]
    fn test_detect_invocation_sources_agentic_heuristic() {
        temp_env::with_var("ANTHROPIC_BEDROCK_AWS_REGION", Some("us-west-2"), || {
            let sources = detect_invocation_sources();
            assert!(sources.contains(&"agentic.unknown".to_string()));
        });
    }

    #[test]
    fn test_detect_invocation_sources_deduplication() {
        temp_env::with_vars(
            [("FLOX_INVOCATION_SOURCE", Some("ci")), ("CI", Some("true"))],
            || {
                let sources = detect_invocation_sources();
                // Should only contain "ci" once despite both explicit and inferred
                assert_eq!(sources.iter().filter(|s| *s == "ci").count(), 1);
            },
        );
    }

    #[test]
    fn test_detect_invocation_sources_nested() {
        temp_env::with_vars(
            [
                ("FLOX_INVOCATION_SOURCE", Some("ci.github-actions")),
                ("ANTHROPIC_BEDROCK_AWS_REGION", Some("us-west-2")),
            ],
            || {
                let sources = detect_invocation_sources();
                assert!(sources.contains(&"ci.github-actions".to_string()));
                assert!(sources.contains(&"agentic.unknown".to_string()));
            },
        );
    }
}
