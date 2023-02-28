pub mod errors;
pub mod guard;

use std::path::Path;

use ::log::debug;
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use walkdir;

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

/// Replace all occurrences of find with replace in a directory or file
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
