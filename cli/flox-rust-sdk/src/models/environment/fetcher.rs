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
    use std::fs;

    use indoc::indoc;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::path_environment::test_helpers::new_path_environment_in;
    #[test]
    fn fetch_relative_path() {
        let (flox, tempdir) = flox_instance();

        let environment_path = tempdir.path().join("environment");
        let manifest_contents = indoc! {r#"
        version = 1
        "#};
        let manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        fs::create_dir(&environment_path).unwrap();
        new_path_environment_in(&flox, manifest_contents, &environment_path);

        let include_fetcher = IncludeFetcher {
            base_directory: Some(tempdir.path().to_path_buf()),
        };

        let include_descriptor = IncludeDescriptor::Local {
            dir: environment_path.file_name().unwrap().into(),
            name: None,
        };

        let fetched = include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        assert_eq!(fetched, LockedInclude {
            manifest,
            name: "environment".to_string(),
            descriptor: include_descriptor,
        })
    }

    #[test]
    fn fetch_absolute_path() {
        let (flox, tempdir) = flox_instance();

        let environment_path = tempdir.path().join("environment");
        let manifest_contents = indoc! {r#"
        version = 1
        "#};
        let manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        fs::create_dir(&environment_path).unwrap();
        new_path_environment_in(&flox, manifest_contents, &environment_path);

        let include_fetcher = IncludeFetcher {
            base_directory: Some(tempdir.path().to_path_buf()),
        };

        let include_descriptor = IncludeDescriptor::Local {
            dir: environment_path,
            name: None,
        };

        let fetched = include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        assert_eq!(fetched, LockedInclude {
            manifest,
            name: "environment".to_string(),
            descriptor: include_descriptor,
        })
    }
}
