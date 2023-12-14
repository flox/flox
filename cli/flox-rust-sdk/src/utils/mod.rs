pub mod errors;
pub mod guard;
pub mod rnix;
use std::path::Path;
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
