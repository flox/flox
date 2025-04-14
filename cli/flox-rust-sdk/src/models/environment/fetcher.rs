use std::path::{Path, PathBuf};

use indoc::formatdoc;

use super::{ConcreteEnvironment, EnvironmentError, open_path};
use crate::flox::Flox;
use crate::models::environment::remote_environment::RemoteEnvironment;
use crate::models::environment::{Environment, ManagedPointer};
use crate::models::environment_ref::EnvironmentRef;
use crate::models::lockfile::{LockedInclude, RecoverableMergeError};
use crate::models::manifest::typed::{IncludeDescriptor, Manifest};

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
        let (manifest, name) = match include_environment {
            IncludeDescriptor::Local { dir, name } => self.fetch_local(flox, dir, name),
            IncludeDescriptor::Remote { remote, name } => self.fetch_remote(flox, remote, name),
        }?;

        Ok(LockedInclude {
            manifest,
            name,
            descriptor: include_environment.clone(),
        })
    }

    /// Fetch a local (path or managed) environment, only if it's already locked.
    fn fetch_local(
        &self,
        flox: &Flox,
        dir: impl AsRef<Path>,
        name: &Option<String>,
    ) -> Result<(Manifest, String), EnvironmentError> {
        if self.base_directory.is_none() {
            return Err(EnvironmentError::Recoverable(
                RecoverableMergeError::RemoteCannotIncludeLocal,
            ));
        };

        let path = self
            .expand_include_dir(dir)
            .map_err(EnvironmentError::Recoverable)?;
        let environment = open_path(flox, &path)?;

        match &environment {
            ConcreteEnvironment::Path(environment) => {
                if !environment.lockfile_up_to_date()? {
                    return Err(EnvironmentError::Recoverable(
                        RecoverableMergeError::Catchall(formatdoc! {"
                            cannot include environment since its manifest and lockfile are out of sync

                            To resolve this issue run 'flox edit -d {}' and retry
                        ", path.to_string_lossy()}.to_string()
                        ),
                    ));
                }
            },
            ConcreteEnvironment::Managed(environment) => {
                if environment.has_local_changes(flox)? {
                    return Err(EnvironmentError::Recoverable(
                        RecoverableMergeError::Catchall(formatdoc! {"
                            cannot include environment since it has changes not yet synced to a generation.

                            To resolve this issue, run either
                            * 'flox edit -d {} --sync' to commit your local changes to a new generation
                            * 'flox edit -d {} --reset' to discard your local changes and reset to the latest generation
                        ", path.to_string_lossy(), path.to_string_lossy()}.to_string())));
                }
            },
            ConcreteEnvironment::Remote(_) => {
                unreachable!("opening a path cannot result in a remote environment");
            },
        }

        let manifest = environment.manifest(flox)?;
        let name = name
            .clone()
            .unwrap_or_else(|| environment.name().to_string());

        Ok((manifest, name))
    }

    /// Fetch a remote environment.
    fn fetch_remote(
        &self,
        flox: &Flox,
        remote: &EnvironmentRef,
        name: &Option<String>,
    ) -> Result<(Manifest, String), EnvironmentError> {
        let pointer =
            ManagedPointer::new(remote.owner().clone(), remote.name().clone(), &flox.floxhub);

        // Don't affect existing open remotes but still uses the same floxmeta.
        let tempdir =
            tempfile::tempdir_in(&flox.temp_dir).map_err(EnvironmentError::CreateTempDir)?;
        let environment = RemoteEnvironment::new_in(flox, tempdir.path(), pointer)?;

        let manifest = environment.manifest(flox)?;
        let name = name
            .clone()
            .unwrap_or_else(|| environment.name().to_string());

        Ok((manifest, name))
    }

    /// For directories that aren't absolute, join them to the base_directory
    /// for this IncludeFetcher
    pub fn expand_include_dir(
        &self,
        dir: impl AsRef<Path>,
    ) -> Result<PathBuf, RecoverableMergeError> {
        let Some(base_directory) = &self.base_directory else {
            return Err(RecoverableMergeError::RemoteCannotIncludeLocal);
        };

        let dir = dir.as_ref();

        Ok(if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            base_directory.join(dir)
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
    use crate::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use crate::models::environment::managed_environment::test_helpers::mock_managed_environment_in;
    use crate::models::environment::path_environment::test_helpers::new_path_environment_in;
    use crate::models::environment::remote_environment::test_helpers::mock_remote_environment;

    #[test]
    fn fetch_path_relative_path() {
        let (flox, tempdir) = flox_instance();

        let environment_path = tempdir.path().join("environment");
        let manifest_contents = indoc! {r#"
        version = 1
        "#};
        let manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        fs::create_dir(&environment_path).unwrap();
        let mut environment = new_path_environment_in(&flox, manifest_contents, &environment_path);
        environment.lockfile(&flox).unwrap();

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
    fn fetch_path_absolute_path() {
        let (flox, tempdir) = flox_instance();

        let environment_path = tempdir.path().join("environment");
        let manifest_contents = indoc! {r#"
        version = 1
        "#};
        let manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        fs::create_dir(&environment_path).unwrap();
        let mut environment = new_path_environment_in(&flox, manifest_contents, &environment_path);
        environment.lockfile(&flox).unwrap();

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

    /// For fetching path environments:
    /// - Fetching fails when not locked
    /// - Fetching succeeds for trivial changes in the manifest (e.g. comments)
    /// - Fetching fails when there are non-trivial changes in the manifest not
    ///   reflected in the lockfile
    #[test]
    fn fetch_path_fails_if_out_of_sync() {
        let (flox, tempdir) = flox_instance();

        let environment_path = tempdir.path().join("environment");
        let manifest_contents = indoc! {r#"
        version = 1
        "#};

        fs::create_dir(&environment_path).unwrap();
        let mut environment = new_path_environment_in(&flox, manifest_contents, &environment_path);

        let include_fetcher = IncludeFetcher {
            base_directory: Some(tempdir.path().to_path_buf()),
        };

        let include_descriptor = IncludeDescriptor::Local {
            dir: environment_path.file_name().unwrap().into(),
            name: None,
        };

        let expected_error = formatdoc! {r#"
        cannot include environment since its manifest and lockfile are out of sync

        To resolve this issue run 'flox edit -d {}' and retry
        "#, environment_path.to_string_lossy()};

        // Fetching should fail before locking
        let err = include_fetcher
            .fetch(&flox, &include_descriptor)
            .unwrap_err();
        assert_eq!(err.to_string(), expected_error);

        // After locking, fetching should succeed
        environment.lockfile(&flox).unwrap();
        include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        // After writing a comment, fetching should succeed
        fs::write(environment.manifest_path(&flox).unwrap(), indoc! {r#"
        version = 1

        # comment
        "#})
        .unwrap();
        include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        // After writing an actual change, fetching should fail
        fs::write(environment.manifest_path(&flox).unwrap(), indoc! {r#"
        version = 1

        # comment
        [vars]
        foo = "bar"
        "#})
        .unwrap();
        let err = include_fetcher
            .fetch(&flox, &include_descriptor)
            .unwrap_err();
        assert_eq!(err.to_string(), expected_error);
    }

    /// fetch() errors if attempting to fetch an out of sync managed environment
    #[test]
    fn fetch_managed_fails_if_out_of_sync() {
        let owner = "owner".parse().unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));

        let environment_path = tempdir.path().join("environment");
        let manifest_contents = indoc! {r#"
        version = 1
        "#};

        fs::create_dir(&environment_path).unwrap();
        let environment =
            mock_managed_environment_in(&flox, manifest_contents, owner, &environment_path, None);

        let include_fetcher = IncludeFetcher {
            base_directory: Some(tempdir.path().to_path_buf()),
        };

        let include_descriptor = IncludeDescriptor::Local {
            dir: environment_path.file_name().unwrap().into(),
            name: None,
        };

        // After writing a comment, fetching should fail
        fs::write(environment.manifest_path(&flox).unwrap(), indoc! {r#"
        version = 1

        # comment
        "#})
        .unwrap();
        let err = include_fetcher
            .fetch(&flox, &include_descriptor)
            .unwrap_err();
        assert!(err.to_string().contains(
            "cannot include environment since it has changes not yet synced to a generation"
        ));
    }

    #[test]
    fn fetch_remote() {
        let env_ref = EnvironmentRef::new("owner", "name").unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(env_ref.owner()));

        let mut remote_env = mock_remote_environment(
            &flox,
            "version = 1",
            env_ref.owner().clone(),
            Some(&env_ref.name().to_string()),
        );

        // Open the remote environment in the default location to simulate an existing activation.
        let open_env = RemoteEnvironment::new(
            &flox,
            ManagedPointer::new(
                env_ref.owner().clone(),
                env_ref.name().clone(),
                &flox.floxhub,
            ),
        )
        .unwrap();
        let open_env_manifest_previous = open_env.manifest(&flox).unwrap();

        // Modify the remote environment with a new generation.
        let manifest_contents = indoc! {r#"
            version = 1

            [vars]
            foo = "bar"
        "#};
        let manifest = toml_edit::de::from_str(manifest_contents).unwrap();
        remote_env
            .edit(&flox, manifest_contents.to_string())
            .unwrap();

        // Fetch and lock the remote environment.
        let include_fetcher = IncludeFetcher {
            base_directory: Some(tempdir.path().to_path_buf()),
        };
        let include_descriptor = IncludeDescriptor::Remote {
            remote: "owner/name".parse().unwrap(),
            name: None,
        };
        let fetched = include_fetcher.fetch(&flox, &include_descriptor).unwrap();
        assert_eq!(
            fetched,
            LockedInclude {
                manifest,
                name: "name".to_string(),
                descriptor: include_descriptor,
            },
            "fetch should get the new generation"
        );

        assert_eq!(
            open_env.manifest(&flox).unwrap(),
            open_env_manifest_previous,
            "fetch should not affect the generation of an already open environment"
        );
    }
}
