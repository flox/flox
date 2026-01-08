use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::flox::Flox;
use crate::models::environment::{ConcreteEnvironment, Environment, EnvironmentError};

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("failed to open manifest at path {}", .0.display())]
    OpenManifest(PathBuf),
    #[error("environment {0} is not writable")]
    NotWritable(String),
    #[error(transparent)]
    EnvironmentError(#[from] EnvironmentError),
}

/// Determines whether a local environment is writable by attempting to open
/// the manifest file with write permissions. Returns Ok(true) if writable,
/// Ok(false) if the file exists and is not writable, or Err(_) if we failed
/// to open the file for some other reason (e.g. it doesn't exist).
fn local_env_is_writable(manifest_path: &Path) -> Result<bool, MigrationError> {
    let maybe_file = std::fs::OpenOptions::new()
        .create(false)
        .write(true)
        .open(manifest_path);
    match maybe_file {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == ErrorKind::PermissionDenied => Ok(false),
        _ => Err(MigrationError::OpenManifest(manifest_path.to_path_buf())),
    }
}

pub fn try_migrate_v1_to_v2(
    flox: &Flox,
    env: &mut ConcreteEnvironment,
) -> Result<(), MigrationError> {
    match env {
        ConcreteEnvironment::Path(inner) => {
            if !local_env_is_writable(inner.manifest_path(flox)?.as_path())? {
                return Err(MigrationError::NotWritable(inner.name().to_string()));
            }
            todo!()
        },
        // You can't check write permissions ahead of time for FloxHub envs
        // because that information is stored server side and a local cache
        // could be invalidated at any time.
        ConcreteEnvironment::Managed(inner) => todo!(),
        ConcreteEnvironment::Remote(inner) => todo!(),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn detects_readonly_and_writable_local_envs() {
        let tempdir = TempDir::new().unwrap();
        let writable_path = tempdir.path().join("writable");
        let readonly_path = tempdir.path().join("readonly");
        let nonexistent_path = tempdir.path().join("does_not_exist");

        // Create the files
        let _writable = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&writable_path)
            .unwrap();
        let readonly = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&readonly_path)
            .unwrap();

        // Make the readonly file readonly
        let mut perms = readonly.metadata().unwrap().permissions();
        perms.set_readonly(true);
        readonly.set_permissions(perms).unwrap();

        // Writable file should return Ok(true)
        assert!(local_env_is_writable(&writable_path).unwrap());

        // Readonly file should return Ok(false)
        assert!(!local_env_is_writable(&readonly_path).unwrap());

        // Nonexistent file should return an error
        assert!(local_env_is_writable(&nonexistent_path).is_err());
    }

    #[test]
    fn detects_writable_remote_env() {
        todo!()
    }

    #[test]
    fn detects_writable_managed_env() {
        todo!()
    }

    #[test]
    fn error_when_manifest_is_v1_but_lockfile_contains_v2() {
        todo!()
    }

    #[test]
    fn writable_v1_env_reported_as_migratable() {
        todo!()
    }

    #[test]
    fn readonly_v1_env_reported_as_not_migratable() {
        todo!()
    }

    #[test]
    fn writable_v2_env_reported_as_no_migration_needed() {
        todo!()
    }

    #[test]
    fn readonly_v2_env_reported_as_no_migration_needed() {
        todo!()
    }

    #[test]
    fn identifies_package_that_needs_migration() {
        todo!()
    }

    #[test]
    fn identifies_package_that_doesnt_need_migration() {
        todo!()
    }

    #[test]
    fn migrated_package_contains_all_outputs() {
        todo!()
    }

    #[test]
    fn package_not_needing_migration_is_untouched() {
        todo!()
    }

    #[test]
    fn migration_updates_manifest_version() {
        todo!()
    }

    #[test]
    fn can_migrate_local_environment() {
        todo!()
    }

    #[test]
    fn can_migrate_remote_environment() {
        todo!()
    }

    #[test]
    fn can_migrate_managed_environment() {
        todo!()
    }

    #[test]
    fn migration_creates_new_generation_for_floxhub_env() {
        todo!()
    }
}
