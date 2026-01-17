use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use toml_edit::{Item, Value};
use tracing::debug;

use crate::data::System;
use crate::flox::Flox;
use crate::models::environment::managed_environment::ManagedEnvironment;
use crate::models::environment::path_environment::PathEnvironment;
use crate::models::environment::remote_environment::RemoteEnvironment;
use crate::models::environment::{ConcreteEnvironment, EditResult, Environment, EnvironmentError};
use crate::models::lockfile::{
    LockedPackage,
    LockedPackageCatalog,
    LockedPackageFlake,
    LockedPackageStorePath,
    Lockfile,
    PackageOutputs,
};
use crate::models::manifest::raw::RawManifest;
use crate::models::manifest::typed::{
    Inner,
    Manifest,
    ManifestPackageDescriptor,
    ManifestVersion,
    PackageDescriptorCatalog,
    PackageDescriptorFlake,
    PackageDescriptorStorePath,
    SetOutputs,
};

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("failed to open manifest at path {}", .0.display())]
    OpenManifest(PathBuf),
    #[error("environment {0} is not writable")]
    NotWritable(String),
    #[error("failed to serialize manifest")]
    SerializeManifest(#[from] toml_edit::ser::Error),
    #[error("migration unexpectedly left manifest unchanged")]
    Unchanged,
    #[error("environment was previously migrated to manifest version 2")]
    PreviouslyMigrated,
    #[error(transparent)]
    EnvironmentError(#[from] EnvironmentError),
    #[error("malformed manifest: {0}")]
    RawManifest(String),
    // This variant is a catch-all for situations where the lockfile and manifest
    // aren't consistent with each other for whatever reason.
    #[error("internal error: {0}")]
    Other(String),
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

/// Indicates the outcome of an error-free migration attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum MigrationResult {
    /// The environment was migrated from v1 to v2.
    Updated,

    /// The environment did not need to be migrated, so no action was taken.
    AlreadyV2,

    /// The feature flag wasn't enabled, so we didn't try.
    FeatureNotEnabled,
}

pub trait MigrateEnv: Environment {
    /// Attempts to determine whether the environment is writable before doing
    /// the migration so that we can skip the migration if we know ahead of time
    /// that it isn't possible.
    ///
    /// Returns Ok(_) if it was possible to learn the answer, and Err(_) if we
    /// encountered an error while determining the answer. For path and managed
    /// environments we use filesystem permissions to know whether the
    /// environment is writable. For remote environments, the local write should
    /// always succeed. For managed and remote environments, you can't know
    /// ahead of time whether the *push* will succeed, so we don't consider
    /// those cases when doing the migration.
    fn is_writable(&self, flox: &Flox) -> Result<bool, MigrationError>;

    /// Attempt to migrate the enviroment from a v1 manifest to a v2 manifest.
    fn migrate_env(&mut self, flox: &Flox) -> Result<MigrationResult, MigrationError> {
        if !flox.features.outputs {
            return Ok(MigrationResult::FeatureNotEnabled);
        }
        // Order of priorities for bailing on a migration:
        // - Manifest version is already 2
        // - Environment is readonly
        //
        // This priority order is a result of needing to ensure that a lockfile
        // exists before migrating. Consider this sequence, which would provide
        // surprising behavior:
        // - v1 manifest, v1 lockfile exist
        // - delete v1 lockfile for some reason
        // - attempt to install a package, which is a write operation
        // - write operation triggers migration
        // - no lockfile present to collect package outputs from, so don't
        //   migrate the `outputs` field of each package
        // - v2 manifest, v2 lockfile _without_ migrated package outputs
        // - packages that previously had all outputs now only have the default
        //   outputs
        //
        // Another scenario to consider:
        // - Someone migrated the environment to v2
        // - Somehow the lockfile is missing
        // - The environment is readonly for the current user
        // - User attempts a write operation
        // - Write operation can't possibly succeed because the environment is
        //   readonly.
        // - Attempted write operation triggers the migration attempt
        // - Migration can't succeed because environment is readonly, regardless
        //   of the fact that we don't *need* to migrate because the environment
        //   is already at v2.
        // - Error should be associated with the impossible write operation
        //   instead of the migration that we don't even need to do in the
        //   first place.
        let existing_manifest = self.manifest(flox)?;
        if existing_manifest.version == ManifestVersion::from(2) {
            debug!("manifest version is 2, skipping migration");
            return Ok(MigrationResult::AlreadyV2);
        }
        match self.is_writable(flox) {
            Ok(false) => {
                debug!("environment is not writable");
                return Err(MigrationError::NotWritable(self.name().to_string()));
            },
            Ok(true) => {
                debug!("environment is writable, proceeding with migration");
            },
            Err(err) => {
                return Err(err);
            },
        }
        // This will ensure that a lockfile exists before we attempt
        // to migrate.
        self.set_migrating(true);
        debug!("ensuring lockfile up to date lockfile exists before migration");
        let lockfile = self.lockfile(flox)?.lockfile();
        let mut raw_manifest = self.raw_manifest(flox)?;
        let migrated_manifest = migrate_manifest_v1_to_v2(&existing_manifest, &lockfile)?;
        update_descriptors_in_raw_manifest(&mut raw_manifest, &migrated_manifest)?;
        let migrated_contents = raw_manifest.to_string();
        let edit_result = self.edit(flox, migrated_contents)?;
        if let EditResult::Unchanged = edit_result {
            return Err(MigrationError::Unchanged);
        }
        self.set_migrating(false);
        Ok(MigrationResult::Updated)
    }
}

impl MigrateEnv for PathEnvironment {
    fn is_writable(&self, flox: &Flox) -> Result<bool, MigrationError> {
        local_env_is_writable(self.manifest_path(flox)?.as_path())
    }
}

impl MigrateEnv for ManagedEnvironment {
    fn is_writable(&self, flox: &Flox) -> Result<bool, MigrationError> {
        local_env_is_writable(self.manifest_path(flox)?.as_path())
    }
}

impl MigrateEnv for RemoteEnvironment {
    fn is_writable(&self, _flox: &Flox) -> Result<bool, MigrationError> {
        Ok(true)
    }
}

impl MigrateEnv for ConcreteEnvironment {
    fn is_writable(&self, flox: &Flox) -> Result<bool, MigrationError> {
        match self {
            ConcreteEnvironment::Path(inner) => inner.is_writable(flox),
            ConcreteEnvironment::Managed(inner) => inner.is_writable(flox),
            ConcreteEnvironment::Remote(inner) => inner.is_writable(flox),
        }
    }
}

fn migrate_manifest_v1_to_v2(
    manifest: &Manifest,
    lockfile: &Lockfile,
) -> Result<Manifest, MigrationError> {
    let mut migrated = manifest.clone();

    // Update the manifest version
    migrated.version = 2.into();

    let collected = collect_locked_packages_by_kind(manifest, lockfile)?;
    let install = migrated.install.inner_mut();
    for locked_descriptor in collected.catalog.iter() {
        install
            .entry(locked_descriptor.install_id.clone())
            .insert_entry(locked_descriptor.migrated());
    }
    for locked_descriptor in collected.flake.iter() {
        install
            .entry(locked_descriptor.install_id.clone())
            .insert_entry(locked_descriptor.migrated());
    }
    // Note: We don't need to migrate store path packages

    Ok(migrated)
}

/// A struct that pairs the concrete package descriptor type with the
/// locked version of that package descriptor type.
///
/// Pairing these together makes it such that anything consuming this type
/// can rely on the fact that they're holding the matching types. In other
/// words, they can rely on the fact that they aren't holding a catalog
/// package descriptor and locked flake package.
struct LockedDescriptor<D, L> {
    install_id: String,
    pd: D,
    locked: HashMap<System, L>,
}

/// Describes the operations necessary to determine whether a locked package
/// descriptor needs to be migrated to list all of its outputs explicitly.
///
/// We determine whether the migration needs to happen by comparing the list
/// of outputs that are available on all systems (e.g. "all" outputs) to the
/// list of outputs that would be installed by default (`outputs_to_install`).
/// If these are the same, then we can save some verbosity in the manifest by
/// not listing outputs for this package.
trait MigratePackage {
    /// Returns true if this locked package descriptor needs to be migrated
    /// to explicitly list out its outputs.
    fn needs_migration(&self) -> bool {
        let all_outputs = self.output_union();
        let outputs_to_install = self.outputs_to_install_union();
        all_outputs != outputs_to_install
    }

    /// Returns the set of outputs that are available on all systems.
    fn output_union(&self) -> HashSet<OutputName>;

    /// Returns a deduplicated set of outputs to install.
    ///
    /// There's currently a bug in catalog-server that lists certain output
    /// names more than once. While this doesn't have any functional effects,
    /// when we go to compare the full list of outputs to the list of outputs
    /// to install, it will cause issues.
    fn outputs_to_install_union(&self) -> HashSet<OutputName>;

    /// Returns the migrated package descriptor.
    ///
    /// This may be a no-op for certain packages (store path packages and
    /// packages where `outputs_to_install` matches the list of all outputs).
    fn migrated(&self) -> ManifestPackageDescriptor;
}

// These two types have exactly the same logic for doing the migration,
// but their `outputs` and `outputs_to_install` fields are nested differently
// on their locked package types, so by adding and using some interfaces we can
// write the logic once for both kinds of package descriptor:
// - PackageDescriptorCatalog
// - PackageDescriptorFlake
impl<P, L> MigratePackage for LockedDescriptor<P, L>
where
    // The package descriptor type
    P: SetOutputs + Into<ManifestPackageDescriptor> + Clone,
    // The locked package type
    L: PackageOutputs,
{
    fn output_union(&self) -> HashSet<OutputName> {
        let initial_outputs = self
            .locked
            .values()
            .next()
            .map(|locked_pkg| locked_pkg.outputs().keys().cloned().collect::<HashSet<_>>())
            .unwrap_or_default();
        self.locked
            .values()
            .fold(initial_outputs, |acc, locked_pkg| {
                let set = locked_pkg.outputs().keys().cloned().collect::<HashSet<_>>();
                acc.union(&set).cloned().collect::<HashSet<_>>()
            })
    }

    fn outputs_to_install_union(&self) -> HashSet<OutputName> {
        let initial_outputs = self
            .locked
            .values()
            .next()
            .map(|locked_pkg| {
                HashSet::from_iter(locked_pkg.outputs_to_install().clone().unwrap_or(vec![]))
            })
            .unwrap_or_default();
        self.locked
            .values()
            .fold(initial_outputs, |acc, locked_pkg| {
                let set =
                    HashSet::from_iter(locked_pkg.outputs_to_install().clone().unwrap_or(vec![]));
                acc.union(&set).cloned().collect::<HashSet<_>>()
            })
    }

    fn migrated(&self) -> ManifestPackageDescriptor {
        let mut pd = self.pd.clone();
        if self.needs_migration() {
            pd.set_outputs_to_all();
        }
        pd.into()
    }
}

/// The pairings between concrete package descriptor types from the manifest,
/// and their locked variants as collected from the lockfile.
struct CollectedPackages {
    catalog: Vec<LockedDescriptor<PackageDescriptorCatalog, LockedPackageCatalog>>,
    flake: Vec<LockedDescriptor<PackageDescriptorFlake, LockedPackageFlake>>,
    // Unused for now, but kept here since the union of all three fields on this
    // struct should form the complete set of packages in the lockfile.
    _store_path: Vec<LockedDescriptor<PackageDescriptorStorePath, LockedPackageStorePath>>,
}

// Between output names, install IDs, systems, etc, which are all strings
// under the hood, having different types to keep them straight makes the
// interfaces a little bit easier to read.
type OutputName = String;

// This function is generic because I want to reuse it for different concrete
// locked package types
/// Finds all [LockedPackage]s corresponding to this install ID. Returns `None`
/// if none were found, otherwise the `Vec` is guaranteed to be non-empty and
/// contain the list of locked packages.
fn get_locked_packages_by_install_id(
    install_id: &str,
    lockfile: &Lockfile,
) -> Option<Vec<LockedPackage>> {
    let pkgs = lockfile
        .packages
        .iter()
        .filter(|p| p.install_id() == install_id)
        .cloned()
        .collect::<Vec<_>>();
    if pkgs.is_empty() { None } else { Some(pkgs) }
}

fn collect_locked_packages_by_kind(
    manifest: &Manifest,
    lockfile: &Lockfile,
) -> Result<CollectedPackages, MigrationError> {
    let mut catalog_pkgs = vec![];
    let mut flake_pkgs = vec![];
    let mut store_path_pkgs = vec![];
    for (install_id, descriptor) in manifest.install.inner().iter() {
        use ManifestPackageDescriptor::*;
        match descriptor {
            Catalog(pd) => {
                let locked = get_locked_packages_by_install_id(install_id, lockfile);
                // Note that since we ensure that the manifest is locked before
                // attempting the migration, you _shouldn't_ see this in real
                // life. We're just being careful and showng a real error message
                // instead of a panic here.
                if locked.is_none() {
                    return Err(MigrationError::Other(format!(
                        "package '{install_id}' in the manifest has no locked packages in the lockfile"
                    )));
                }
                let locked = locked.unwrap().into_iter().map(|p| {
                    if let LockedPackage::Catalog(ref pkg) = p {
                        Ok((p.system().clone(), pkg.clone()))
                    } else {
                        Err(MigrationError::Other(format!("package '{install_id}' was a catalog package in the manifest, but not in the lockfile")))
                    }
                }).collect::<Result<HashMap<_, _>, MigrationError>>()?;
                let locked_descriptor = LockedDescriptor {
                    install_id: install_id.to_string(),
                    pd: pd.clone(),
                    locked,
                };
                catalog_pkgs.push(locked_descriptor);
            },
            FlakeRef(pd) => {
                let locked = get_locked_packages_by_install_id(install_id, lockfile);
                // Note that since we ensure that the manifest is locked before
                // attempting the migration, you _shouldn't_ see this in real
                // life. We're just being careful and showng a real error message
                // instead of a panic here.
                if locked.is_none() {
                    return Err(MigrationError::Other(format!(
                        "package '{install_id}' in the manifest has no locked packages in the lockfile"
                    )));
                }
                let locked = locked.unwrap().into_iter().map(|p| {
                    if let LockedPackage::Flake(ref pkg) = p {
                        Ok((p.system().clone(), pkg.clone()))
                    } else {
                        Err(MigrationError::Other(format!("package '{install_id}' was a flake package in the manifest, but not in the lockfile")))
                    }
                }).collect::<Result<HashMap<_, _>, MigrationError>>()?;
                let locked_descriptor = LockedDescriptor {
                    install_id: install_id.to_string(),
                    pd: pd.clone(),
                    locked,
                };
                flake_pkgs.push(locked_descriptor);
            },
            StorePath(pd) => {
                let locked = get_locked_packages_by_install_id(install_id, lockfile);
                // Note that since we ensure that the manifest is locked before
                // attempting the migration, you _shouldn't_ see this in real
                // life. We're just being careful and showng a real error message
                // instead of a panic here.
                if locked.is_none() {
                    return Err(MigrationError::Other(format!(
                        "package '{install_id}' in the manifest has no locked packages in the lockfile"
                    )));
                }
                let locked = locked.unwrap().into_iter().map(|p| {
                    if let LockedPackage::StorePath(ref pkg) = p {
                        Ok((p.system().clone(), pkg.clone()))
                    } else {
                        Err(MigrationError::Other(format!("package '{install_id}' was a store path in the manifest, but not in the lockfile")))
                    }
                }).collect::<Result<HashMap<_, _>, MigrationError>>()?;
                let locked_descriptor = LockedDescriptor {
                    install_id: install_id.to_string(),
                    pd: pd.clone(),
                    locked,
                };
                store_path_pkgs.push(locked_descriptor);
            },
        }
    }
    let collected = CollectedPackages {
        catalog: catalog_pkgs,
        flake: flake_pkgs,
        _store_path: store_path_pkgs,
    };
    Ok(collected)
}

fn update_descriptors_in_raw_manifest(
    raw: &mut RawManifest,
    migrated: &Manifest,
) -> Result<(), MigrationError> {
    raw["version"] = toml_edit::Item::Value(Value::from(*migrated.version.inner() as i64));
    for (install_id, pd) in migrated.install.inner().iter() {
        use ManifestPackageDescriptor::*;
        match pd {
            Catalog(pd_inner) => {
                debug!(id = install_id, "migrating package in raw manifest");
                if let Some(ref outputs) = pd_inner.outputs {
                    raw["install"][install_id]["outputs"] = Item::Value(outputs.to_toml());
                    debug!(id = install_id, "updated outputs");
                } else {
                    debug!(id = install_id, "skipping, no outputs specified");
                }
            },
            FlakeRef(pd_inner) => {
                debug!(id = install_id, "migrating package in raw manifest");
                if let Some(ref outputs) = pd_inner.outputs {
                    raw["install"][install_id]["outputs"] = Item::Value(outputs.to_toml());
                    debug!(id = install_id, "updated outputs");
                } else {
                    debug!(id = install_id, "skipping, no outputs specified");
                }
            },
            _ => {
                debug!(id = install_id, "skipping store path package");
            },
        }
    }
    Ok(())
}

// fn add_outputs_to_toml_descriptor()

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;
    use std::str::FromStr;

    use flox_core::canonical_path::CanonicalPath;
    use tempfile::TempDir;

    use super::*;
    use crate::flox::EnvironmentOwner;
    use crate::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use crate::models::environment::managed_environment::test_helpers::mock_managed_environment_from_env_files;
    use crate::models::environment::path_environment::test_helpers::new_path_environment_from_env_files;
    use crate::models::environment::remote_environment::test_helpers::mock_remote_environment_from_env_files;
    use crate::models::lockfile::LockResult;
    use crate::models::manifest::typed::SelectedOutputs;
    use crate::providers::buildenv::test_helpers::{
        locked_package_catalog_from_mock_all_systems,
        locked_package_flake_from_mock_all_systems,
    };
    use crate::providers::catalog::GENERATED_DATA;
    use crate::providers::catalog::test_helpers::catalog_replay_client;

    fn locked_catalog_descriptor_from_mock(
        install_id: &str,
    ) -> LockedDescriptor<PackageDescriptorCatalog, LockedPackageCatalog> {
        let subpath = format!("envs/{install_id}/manifest.lock");
        let (descriptor, locked_packages) =
            locked_package_catalog_from_mock_all_systems(install_id, GENERATED_DATA.join(subpath));
        let locked_descriptors_by_system = locked_packages
            .into_iter()
            .map(|p| (p.system.clone(), p))
            .collect::<HashMap<_, _>>();
        LockedDescriptor {
            install_id: install_id.to_string(),
            pd: descriptor,
            locked: locked_descriptors_by_system,
        }
    }

    fn locked_flake_descriptor_from_mock(
        install_id: &str,
    ) -> LockedDescriptor<PackageDescriptorFlake, LockedPackageFlake> {
        let subpath = format!("envs/flake_{install_id}/manifest.lock");
        let (descriptor, locked_packages) =
            locked_package_flake_from_mock_all_systems(install_id, GENERATED_DATA.join(subpath));
        let locked_descriptors_by_system = locked_packages
            .into_iter()
            .map(|p| (p.locked_installable.system.clone(), p))
            .collect::<HashMap<_, _>>();
        LockedDescriptor {
            install_id: install_id.to_string(),
            pd: descriptor,
            locked: locked_descriptors_by_system,
        }
    }

    fn catalog_package_with_different_outputs_to_install()
    -> LockedDescriptor<PackageDescriptorCatalog, LockedPackageCatalog> {
        locked_catalog_descriptor_from_mock("bash")
    }

    fn catalog_package_with_same_outputs_as_outputs_to_install()
    -> LockedDescriptor<PackageDescriptorCatalog, LockedPackageCatalog> {
        locked_catalog_descriptor_from_mock("hello")
    }

    fn flake_package_with_different_outputs_to_install()
    -> LockedDescriptor<PackageDescriptorFlake, LockedPackageFlake> {
        locked_flake_descriptor_from_mock("bash")
    }

    fn flake_package_with_same_outputs_as_outputs_to_install()
    -> LockedDescriptor<PackageDescriptorFlake, LockedPackageFlake> {
        locked_flake_descriptor_from_mock("hello")
    }

    #[test]
    fn detects_readonly_and_writable_local_env_files() {
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
        // Make the readonly file writable so it can be deleted
        let mut perms = readonly.metadata().unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        readonly.set_permissions(perms).unwrap();

        // Nonexistent file should return an error
        assert!(local_env_is_writable(&nonexistent_path).is_err());
    }

    #[test]
    fn detects_writable_path_env() {
        let (mut flox, _tmpdir) = flox_instance();
        flox.features.outputs = true;
        let env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        assert!(env.is_writable(&flox).unwrap());
    }

    #[test]
    fn detects_writable_managed_env() {
        let env_owner = EnvironmentOwner::from_str("owner").unwrap();
        let (mut flox, _tmpdir) = flox_instance_with_optional_floxhub(Some(&env_owner));
        flox.features.outputs = true;
        let env = mock_managed_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs/hello"),
            env_owner.clone(),
        );
        assert!(env.is_writable(&flox).unwrap());
    }

    #[test]
    fn detects_readonly_path_env() {
        let (mut flox, _tmpdir) = flox_instance();
        flox.features.outputs = true;
        let env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        let manifest_path = env.manifest_path(&flox).unwrap();
        let mut perms = manifest_path.metadata().unwrap().permissions();
        perms.set_readonly(true);
        let manifest_file = OpenOptions::new()
            .write(true)
            .create_new(false)
            .open(&manifest_path)
            .unwrap();
        manifest_file.set_permissions(perms).unwrap();

        assert!(!env.is_writable(&flox).unwrap());

        // Make the readonly file writable so it can be deleted
        let mut perms = manifest_path.metadata().unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        manifest_file.set_permissions(perms).unwrap();
    }

    #[test]
    fn detects_readonly_managed_env() {
        let env_owner = EnvironmentOwner::from_str("owner").unwrap();
        let (mut flox, _tmpdir) = flox_instance_with_optional_floxhub(Some(&env_owner));
        flox.features.outputs = true;
        let env = mock_managed_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs/hello"),
            env_owner.clone(),
        );

        let manifest_path = env.manifest_path(&flox).unwrap();
        let mut perms = manifest_path.metadata().unwrap().permissions();
        perms.set_readonly(true);
        let manifest_file = OpenOptions::new()
            .write(true)
            .create_new(false)
            .open(&manifest_path)
            .unwrap();
        manifest_file.set_permissions(perms).unwrap();

        assert!(!env.is_writable(&flox).unwrap());

        // Make the readonly file writable so it can be deleted
        let mut perms = manifest_path.metadata().unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        manifest_file.set_permissions(perms).unwrap();
    }

    // NOTE: There's no test for writable remote environments because they're
    //       always writable locally.

    #[test]
    fn readonly_v1_env_reported_as_not_migratable() {
        let (mut flox, _tmpdir) = flox_instance();
        flox.features.outputs = true;
        let mut env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        let manifest_path = env.manifest_path(&flox).unwrap();
        let mut perms = manifest_path.metadata().unwrap().permissions();
        perms.set_readonly(true);
        let manifest_file = OpenOptions::new()
            .write(true)
            .create_new(false)
            .open(&manifest_path)
            .unwrap();
        manifest_file.set_permissions(perms).unwrap();

        let res = env.migrate_env(&flox);
        assert!(matches!(res, Err(MigrationError::NotWritable(_))));

        // Make the readonly file writable so it can be deleted
        let mut perms = manifest_path.metadata().unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        manifest_file.set_permissions(perms).unwrap();
    }

    #[test]
    fn writable_v2_env_reported_as_no_migration_needed() {
        let (mut flox, _tmpdir) = flox_instance();
        flox.features.outputs = true;
        let mut env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        // Update the manifest to version 2 so no migration is needed
        let mut manifest = env.manifest(&flox).unwrap();
        manifest.version = 2.into();
        env.edit(&flox, toml_edit::ser::to_string(&manifest).unwrap())
            .unwrap();

        let res = env.migrate_env(&flox).unwrap();
        assert_eq!(res, MigrationResult::AlreadyV2);
    }

    #[test]
    fn readonly_v2_env_reported_as_no_migration_needed() {
        let (mut flox, _tmpdir) = flox_instance();
        flox.features.outputs = true;
        let mut env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        // Update the manifest to version 2 so no migration is needed
        let mut manifest = env.manifest(&flox).unwrap();
        manifest.version = 2.into();
        env.edit(&flox, toml_edit::ser::to_string(&manifest).unwrap())
            .unwrap();

        // Make the manifest readonly
        let manifest_path = env.manifest_path(&flox).unwrap();
        let mut perms = manifest_path.metadata().unwrap().permissions();
        perms.set_readonly(true);
        let manifest_file = OpenOptions::new()
            .write(true)
            .create_new(false)
            .open(&manifest_path)
            .unwrap();
        manifest_file.set_permissions(perms).unwrap();

        let res = env.migrate_env(&flox).unwrap();
        assert_eq!(res, MigrationResult::AlreadyV2);

        // Make the readonly file writable so it can be deleted
        let mut perms = manifest_path.metadata().unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        manifest_file.set_permissions(perms).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn v1_with_missing_lockfile_is_locked_during_migration() {
        // This test originally verified that the missing lockfile was produced
        // as part of pre-checks before the migration, and that is still the
        // actual behavior, but the interface was rewritten in such a way that
        // it's not possible to verify that the lockfile is produced *before*
        // migrating (there's no place to pause and check), so instead we verify
        // that the migration at least *produces* a lockfile.
        let (mut flox, _tmpdir) = flox_instance();
        let mut env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        flox.features.outputs = true;
        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("envs/hello/hello.yaml")).await;

        std::fs::remove_file(env.lockfile_path(&flox).unwrap()).unwrap();

        env.migrate_env(&flox).unwrap();
        assert!(env.lockfile_path(&flox).unwrap().exists());
    }

    #[test]
    fn lock_of_v1_with_up_to_date_lockfile_doesnt_trigger_migration() {
        let (mut flox, _tmpdir) = flox_instance();
        let mut env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        flox.features.outputs = true;
        let res = env.lockfile(&flox).unwrap();
        assert!(matches!(res, LockResult::Unchanged(_)));
        let manifest = res.lockfile().manifest;
        assert_eq!(manifest.version, 1.into());
    }

    #[test]
    fn looks_up_locked_packages_by_install_id() {
        let lockfile_path =
            CanonicalPath::new_unchecked(GENERATED_DATA.join("envs/bash/manifest.lock"));
        let lockfile = Lockfile::read_from_file(&lockfile_path).unwrap();

        // A package we know exists
        let pkgs = get_locked_packages_by_install_id("bash", &lockfile).unwrap();
        assert_eq!(pkgs.len(), 4);

        // A package we know doesn't exist
        let should_be_none = get_locked_packages_by_install_id("foo", &lockfile);
        assert!(should_be_none.is_none());
    }

    #[test]
    fn collects_all_packages() {
        // I've chosen this environment for a few reasons:
        // - It contains multiple packages
        // - Some packages are restricted to specific systems
        // - The Linux packages have the duplicate output bug from the catalog
        //   server e.g. `outputs_to_install = [ "man", "out", "out", "out"]`.
        // - Some packages (`gnumake`) don't have the same outputs for all
        //   systems.
        //
        // (note: "oti" = "outputs to install")
        //
        // Lockfile contents:
        // - nodejs: all systems
        //     - outputs: dev, libv8, out
        //     - oti: out
        // - python3: all systems
        //     - outputs: out, (debug, on *-linux)
        //     - oti: out
        // - gnumake: all systems
        //     - outputs: info, man, out, (debug, on *-linux)
        //     - oti: man, out
        // - clang: *-darwin
        //     - outputs: out
        //     - oti: out
        // - cctools: *-darwin
        //     - outputs: dev, gas, libtool, man, out
        //     - oti: man, out
        // - libcxx: *-darwin
        //     - outputs: dev, out
        //     - oti: out
        // - gcc: *-linux
        //     - outputs: info, man, out
        //     - oti: man, out
        let manifest_path = GENERATED_DATA.join("envs/krb5_prereqs/manifest.toml");
        let contents = std::fs::read_to_string(manifest_path).unwrap();
        let manifest = Manifest::from_str(&contents).unwrap();
        let lockfile_path =
            CanonicalPath::new_unchecked(GENERATED_DATA.join("envs/krb5_prereqs/manifest.lock"));
        let lockfile = Lockfile::read_from_file(&lockfile_path).unwrap();

        let collected = collect_locked_packages_by_kind(&manifest, &lockfile).unwrap();

        assert_eq!(collected.catalog.len(), 7);
        assert_eq!(collected.flake.len(), 0);
        assert_eq!(collected._store_path.len(), 0);

        let nodejs = collected
            .catalog
            .iter()
            .find(|p| p.install_id.as_str() == "nodejs")
            .unwrap();
        assert_eq!(nodejs.locked.len(), 4);

        let python3 = collected
            .catalog
            .iter()
            .find(|p| p.install_id.as_str() == "python3")
            .unwrap();
        assert_eq!(python3.locked.len(), 4);

        let gnumake = collected
            .catalog
            .iter()
            .find(|p| p.install_id.as_str() == "make")
            .unwrap();
        assert_eq!(gnumake.locked.len(), 4);

        let clang = collected
            .catalog
            .iter()
            .find(|p| p.install_id.as_str() == "clang")
            .unwrap();
        assert_eq!(clang.locked.len(), 2);

        let cctools = collected
            .catalog
            .iter()
            .find(|p| p.install_id.as_str() == "cctools")
            .unwrap();
        assert_eq!(cctools.locked.len(), 2);

        let libcxx = collected
            .catalog
            .iter()
            .find(|p| p.install_id.as_str() == "libcxx")
            .unwrap();
        assert_eq!(libcxx.locked.len(), 2);

        let gcc = collected
            .catalog
            .iter()
            .find(|p| p.install_id.as_str() == "gcc")
            .unwrap();
        assert_eq!(gcc.locked.len(), 2);
    }

    #[test]
    fn identifies_catalog_package_that_needs_migration() {
        let locked_pd = catalog_package_with_different_outputs_to_install();
        assert!(locked_pd.needs_migration());
    }

    #[test]
    fn identifies_catalog_package_that_doesnt_need_migration() {
        let locked_pd = catalog_package_with_same_outputs_as_outputs_to_install();
        assert!(!locked_pd.needs_migration());
    }

    #[test]
    fn identifies_flake_package_that_needs_migration() {
        let locked_pd = flake_package_with_different_outputs_to_install();
        assert!(locked_pd.needs_migration());
    }

    #[test]
    fn identifies_flake_package_that_doesnt_need_migration() {
        let locked_pd = flake_package_with_same_outputs_as_outputs_to_install();
        assert!(!locked_pd.needs_migration());
    }

    #[test]
    fn migrated_package_contains_all_outputs() {
        let needs_migration = catalog_package_with_different_outputs_to_install();
        let migrated = needs_migration.migrated();
        let ManifestPackageDescriptor::Catalog(pd) = migrated else {
            panic!("expected catalog package");
        };
        assert_eq!(pd.outputs, Some(SelectedOutputs::all()));
    }

    #[test]
    fn package_not_needing_migration_is_untouched() {
        let locked_descriptor = catalog_package_with_same_outputs_as_outputs_to_install();
        let migrated = locked_descriptor.migrated();
        let ManifestPackageDescriptor::Catalog(pd) = migrated else {
            panic!("expected catalog package");
        };
        assert_eq!(pd.outputs, None);
    }

    #[test]
    fn migration_updates_manifest_version() {
        let manifest_path = GENERATED_DATA.join("envs/krb5_prereqs/manifest.toml");
        let contents = std::fs::read_to_string(manifest_path).unwrap();
        let manifest = Manifest::from_str(&contents).unwrap();
        let lockfile_path =
            CanonicalPath::new_unchecked(GENERATED_DATA.join("envs/krb5_prereqs/manifest.lock"));
        let lockfile = Lockfile::read_from_file(&lockfile_path).unwrap();

        let migrated = migrate_manifest_v1_to_v2(&manifest, &lockfile).unwrap();
        assert_eq!(migrated.version, 2.into());
    }

    #[test]
    fn can_migrate_local_environment() {
        let (mut flox, _tmpdir) = flox_instance();
        let mut env =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/krb5_prereqs"));
        flox.features.outputs = true;
        env.migrate_env(&flox).unwrap();
        let manifest = env.manifest(&flox).unwrap();
        assert_eq!(manifest.version, 2.into());
        assert_eq!(
            manifest
                .install
                .inner()
                .get("nodejs")
                .unwrap()
                .as_catalog_descriptor_ref()
                .unwrap()
                .outputs
                .as_ref()
                .unwrap(),
            &SelectedOutputs::all()
        );
    }

    #[test]
    fn can_migrate_remote_environment() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (mut flox, _tmpdir) = flox_instance_with_optional_floxhub(Some(&owner));
        flox.features.outputs = true;
        let mut env = mock_remote_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs/krb5_prereqs"),
            owner.clone(),
        );
        let res = env.migrate_env(&flox).unwrap();
        let migrated = env.manifest(&flox).unwrap();

        assert_eq!(res, MigrationResult::Updated);
        assert_eq!(migrated.version, 2.into());
        // Arbitrarily selected package out of the many in this environment
        let ManifestPackageDescriptor::Catalog(nodejs) =
            migrated.install.inner().get("nodejs").unwrap()
        else {
            panic!("expected catalog package descriptor");
        };
        assert_eq!(nodejs.outputs, Some(SelectedOutputs::all()));
    }

    #[test]
    fn can_migrate_managed_environment() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (mut flox, _tmpdir) = flox_instance_with_optional_floxhub(Some(&owner));
        flox.features.outputs = true;
        let mut env = mock_managed_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs/krb5_prereqs"),
            owner.clone(),
        );
        let res = env.migrate_env(&flox).unwrap();
        let migrated = env.manifest(&flox).unwrap();

        assert_eq!(res, MigrationResult::Updated);
        assert_eq!(migrated.version, 2.into());
        // Arbitrarily selected package out of the many in this environment
        let ManifestPackageDescriptor::Catalog(nodejs) =
            migrated.install.inner().get("nodejs").unwrap()
        else {
            panic!("expected catalog package descriptor");
        };
        assert_eq!(nodejs.outputs, Some(SelectedOutputs::all()));
    }

    #[test]
    fn migration_creates_new_generation_for_floxhub_env() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (mut flox, _tmpdir) = flox_instance_with_optional_floxhub(Some(&owner));
        flox.features.outputs = true;
        let mut env = mock_managed_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs/krb5_prereqs"),
            owner.clone(),
        );
        let n_generations_before = env.generations().metadata().unwrap().generations().len();
        env.migrate_env(&flox).unwrap();
        env.manifest(&flox).unwrap();
        let n_generations_after = env.generations().metadata().unwrap().generations().len();

        assert!(n_generations_after > n_generations_before);
    }
}
