pub mod errors;
pub mod guard;
use std::fmt::Display;
use std::path::Path;
use std::time::SystemTime;
use std::{fs, io};

use ::log::debug;
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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

/// Replace all occurrences of `find` with `replace` in a directory or file
pub async fn find_and_replace(
    path: &Path,
    find: &str,
    replace: &str,
) -> Result<(), FindAndReplaceError> {
    for entry in walkdir::WalkDir::new(path) {
        let entry = entry.map_err(FindAndReplaceError::WalkDir)?;
        if entry.file_type().is_file() {
            let mut file = match tokio::fs::File::open(entry.path()).await {
                Ok(f) => f,
                Err(err) => return Err(FindAndReplaceError::OpenTemplateFile(err)),
            };

            let mut file_contents = String::new();
            file.read_to_string(&mut file_contents)
                .await
                .map_err(FindAndReplaceError::ReadTemplateFile)?;

            // TODO optimize with find or something?
            if file_contents.contains(find) {
                let new_contents = file_contents.replace(find, replace);
                match OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(entry.path())
                    .await
                {
                    Ok(mut f) => f
                        .write_all(new_contents.as_bytes())
                        .await
                        .map_err(FindAndReplaceError::WriteTemplateFile)?,
                    Err(err) => return Err(FindAndReplaceError::OpenTemplateFile(err)),
                };
            }
        } else {
            debug!(
                "Skipping entry that is not a regular file: {}",
                entry.path().to_string_lossy()
            );
        }
    }

    Ok(())
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
pub(crate) trait CommandExt {
    /// Provide a [DisplayCommand] that can be used to display
    /// POSIX like formatting of the command.
    fn display(&self) -> DisplayCommand;
}

impl CommandExt for std::process::Command {
    fn display(&self) -> DisplayCommand {
        DisplayCommand(self)
    }
}

pub(crate) struct DisplayCommand<'a>(&'a std::process::Command);

impl Display for DisplayCommand<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let command = self.0;

        let args = command
            .get_args()
            .map(|a| shell_escape::escape(a.to_string_lossy()));

        write!(f, "{}", command.get_program().to_string_lossy())?;
        for arg in args {
            write!(f, " {}", arg)?;
        }

        Ok(())
    }
}
