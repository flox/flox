pub mod errors;
pub mod gomap;
pub mod guard;
pub mod logging;

use std::fmt::{Display, Write};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::SystemTime;
use std::{fs, io};

pub use flox_core::traceable_path;
use thiserror::Error;
use tracing::{debug, trace};
use walkdir;

use self::errors::IoError;

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
    fn display(&self) -> DisplayCommand;
}

impl CommandExt for std::process::Command {
    fn display(&self) -> DisplayCommand {
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
                        tap_fn(&mut context, line.trim_end());

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

/// Returns a `tracing`-compatible form of an `Option<PathBuf>`
pub fn maybe_traceable_path(maybe_path: &Option<PathBuf>) -> impl tracing::Value {
    if let Some(ref p) = maybe_path {
        p.display().to_string()
    } else {
        String::from("null")
    }
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
}
