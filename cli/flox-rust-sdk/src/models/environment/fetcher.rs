use std::path::PathBuf;

use super::{open_path, EnvironmentError};
use crate::flox::Flox;
use crate::models::environment::Environment;
use crate::models::lockfile::{LockedInclude, RecoverableMergeError};
use crate::models::manifest::typed::IncludeDescriptor;

/// Context required to fetch an environment include
#[derive(Clone, Debug)]
pub struct IncludeFetcher {
    pub base_directory: Option<PathBuf>,
}

impl IncludeFetcher {
    pub fn fetch(
        &self,
        flox: &Flox,
        include_environment: &IncludeDescriptor,
    ) -> Result<LockedInclude, EnvironmentError> {
        let Some(base_directory) = &self.base_directory else {
            return Err(EnvironmentError::Recoverable(
                RecoverableMergeError::Catchall(
                    "cannot include environments in remote environments".to_string(),
                ),
            ));
        };
        let (name, environment) = match include_environment {
            IncludeDescriptor::Local { dir, name } => {
                let path = if dir.is_absolute() {
                    dir
                } else {
                    &base_directory.join(dir)
                };
                (name, open_path(flox, path).unwrap())
            },
        };

        // TODO: error if manifest and lock are not in sync

        Ok(LockedInclude {
            manifest: environment.manifest(flox)?,
            name: name
                .clone()
                .unwrap_or_else(|| environment.name().to_string()),
            descriptor: include_environment.clone(),
        })
    }
}

pub mod test_helpers {
    use super::*;

    /// Returns an IncludeFetcher that fails to fetch anything
    pub fn mock_include_fetcher() -> IncludeFetcher {
        IncludeFetcher {
            base_directory: None,
        }
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn fetch_relative_path() {}

    #[test]
    fn fetch_absolute_path() {}
}
