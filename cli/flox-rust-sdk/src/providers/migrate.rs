use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::data::System;
use crate::flox::Flox;
use crate::models::environment::{ConcreteEnvironment, EditResult, Environment, EnvironmentError};
use crate::models::lockfile::{
    LockedPackage,
    LockedPackageCatalog,
    LockedPackageFlake,
    LockedPackageStorePath,
    Lockfile,
};
use crate::models::manifest::typed::{
    Inner,
    Manifest,
    ManifestPackageDescriptor,
    PackageDescriptorCatalog,
    PackageDescriptorFlake,
    PackageDescriptorStorePath,
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

pub fn try_migrate_v1_to_v2(
    flox: &Flox,
    env: &mut ConcreteEnvironment,
) -> Result<(), MigrationError> {
    match env {
        ConcreteEnvironment::Path(inner) => {
            if !local_env_is_writable(inner.manifest_path(flox)?.as_path())? {
                return Err(MigrationError::NotWritable(inner.name().to_string()));
            }
            // We need to make sure that there's a lockfile present so that we
            // can inspect the outputs of each package. We want to avoid this
            // sequence, which could give surprising behavior:
            // - v1 manifest, v1 lockfile exist
            // - delete v1 lockfile for some reason
            // - activate, which locks, which is a write operation
            // - triggers migration
            // - v2 manifest, v2 lockfile _without_ migrated package outputs
            let lockfile = inner
                .as_core_environment_mut()?
                .ensure_locked(flox)?
                .lockfile();
            let existing_manifest = inner.manifest(flox)?;
            let migrated_manifest = migrate_manifest_v1_to_v2(&existing_manifest, &lockfile)?;
            let migrated_contents = toml_edit::ser::to_string(&migrated_manifest)
                .map_err(MigrationError::SerializeManifest)?;
            let edit_result = inner.edit(flox, migrated_contents)?;
            if let EditResult::Unchanged = edit_result {
                return Err(MigrationError::Unchanged);
            }
            Ok(())
        },
        // You can't check write permissions ahead of time for FloxHub envs
        // because that information is stored server side and a local cache
        // could be invalidated at any time.
        ConcreteEnvironment::Managed(inner) => todo!(),
        ConcreteEnvironment::Remote(inner) => todo!(),
    }
}

fn migrate_manifest_v1_to_v2(
    manifest: &Manifest,
    lockfile: &Lockfile,
) -> Result<Manifest, MigrationError> {
    let mut migrated = manifest.clone();

    // Update the manifest version
    migrated.version = 2.into();
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
trait PackageNeedsMigration {
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
}

impl PackageNeedsMigration for LockedDescriptor<PackageDescriptorCatalog, LockedPackageCatalog> {
    fn output_union(&self) -> HashSet<OutputName> {
        let initial_outputs = self
            .locked
            .values()
            .next()
            .map(|locked_pkg| locked_pkg.outputs.keys().cloned().collect::<HashSet<_>>())
            .unwrap_or_default();
        self.locked
            .values()
            .fold(initial_outputs, |acc, locked_pkg| {
                let set = locked_pkg.outputs.keys().cloned().collect::<HashSet<_>>();
                acc.union(&set).cloned().collect::<HashSet<_>>()
            })
    }

    fn outputs_to_install_union(&self) -> HashSet<OutputName> {
        let initial_outputs = self
            .locked
            .values()
            .next()
            .map(|locked_pkg| {
                HashSet::from_iter(locked_pkg.outputs_to_install.clone().unwrap_or(vec![]))
            })
            .unwrap_or_default();
        self.locked
            .values()
            .fold(initial_outputs, |acc, locked_pkg| {
                let set =
                    HashSet::from_iter(locked_pkg.outputs_to_install.clone().unwrap_or(vec![]));
                acc.union(&set).cloned().collect::<HashSet<_>>()
            })
    }
}

impl PackageNeedsMigration for LockedDescriptor<PackageDescriptorFlake, LockedPackageFlake> {
    fn output_union(&self) -> HashSet<OutputName> {
        let initial_outputs = self
            .locked
            .values()
            .next()
            .map(|locked_pkg| {
                locked_pkg
                    .locked_installable
                    .outputs
                    .keys()
                    .cloned()
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        self.locked
            .values()
            .fold(initial_outputs, |acc, locked_pkg| {
                let set = locked_pkg
                    .locked_installable
                    .outputs
                    .keys()
                    .cloned()
                    .collect::<HashSet<_>>();
                acc.union(&set).cloned().collect::<HashSet<_>>()
            })
    }

    fn outputs_to_install_union(&self) -> HashSet<OutputName> {
        let initial_outputs = self
            .locked
            .values()
            .next()
            .map(|locked_pkg| {
                HashSet::from_iter(
                    locked_pkg
                        .locked_installable
                        .outputs_to_install
                        .clone()
                        .unwrap_or(vec![]),
                )
            })
            .unwrap_or_default();
        self.locked
            .values()
            .fold(initial_outputs, |acc, locked_pkg| {
                let set = HashSet::from_iter(
                    locked_pkg
                        .locked_installable
                        .outputs_to_install
                        .clone()
                        .unwrap_or(vec![]),
                );
                acc.union(&set).cloned().collect::<HashSet<_>>()
            })
    }
}

impl PackageNeedsMigration
    for LockedDescriptor<PackageDescriptorStorePath, LockedPackageStorePath>
{
    fn needs_migration(&self) -> bool {
        false
    }

    fn output_union(&self) -> HashSet<OutputName> {
        // SAFETY: this is only called by needs_migration,
        //         which has a hardcoded return value
        unreachable!()
    }

    fn outputs_to_install_union(&self) -> HashSet<OutputName> {
        // SAFETY: this is only called by needs_migration,
        //         which has a hardcoded return value
        unreachable!()
    }
}

trait MigratePackage {
    fn migrate(&self) -> ManifestPackageDescriptor;
}

/// The pairings between concrete package descriptor types from the manifest,
/// and their locked variants as collected from the lockfile.
struct CollectedPackages {
    catalog: Vec<LockedDescriptor<PackageDescriptorCatalog, LockedPackageCatalog>>,
    flake: Vec<LockedDescriptor<PackageDescriptorFlake, LockedPackageFlake>>,
    store_path: Vec<LockedDescriptor<PackageDescriptorStorePath, LockedPackageStorePath>>,
}

// Between output names, install IDs, systems, etc, which are all strings
// under the hood, having different types to keep them straight makes the
// interfaces a little bit easier to read.
type OutputName = String;

/// The organized outputs from all of the single-system locked packages
/// corresponding to a single install ID.
struct CollectedOutputs {
    /// A copy of the `outputs_to_install` field used to help order the
    /// outputs as listed in the user's manifest.
    ///
    /// The order in which the outputs are listed determines a priority
    /// order when rendering the environment, so the order is significant.
    /// The `outputs_to_install` field is a list, which has an ordering.
    /// The `outputs` field is a map, which doesn't have an ordering.
    /// So what we can do to best preserve the intended ordering is put the
    /// outputs from `outputs_to_install` first, and all others afterwards.
    outputs_to_install: Vec<OutputName>,

    /// The output names that appeared for all systems.
    common: HashSet<OutputName>,

    /// The outputs that only appeared for certain systems.
    system_specific: HashMap<OutputName, Vec<System>>,
}

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
        store_path: store_path_pkgs,
    };
    Ok(collected)
}

struct MigratedPackageDescriptor {
    /// The new package descriptor with `outputs` populated.
    pd: ManifestPackageDescriptor,

    /// Any outputs that weren't common to all systems, mapping each output
    /// name to the systems on which it _was_ available.
    ommitted_outputs: HashMap<OutputName, Vec<System>>,
}

fn migrate_package_descriptor_v1_to_v2(
    pd: &ManifestPackageDescriptor,
    lockfile: &Lockfile,
) -> MigratedPackageDescriptor {
    todo!()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use flox_core::canonical_path::CanonicalPath;
    use tempfile::TempDir;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::path_environment::test_helpers::new_path_environment_from_env_files;
    use crate::providers::buildenv::test_helpers::locked_package_catalog_from_mock_all_systems;
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

    fn package_with_different_outputs_to_install()
    -> LockedDescriptor<PackageDescriptorCatalog, LockedPackageCatalog> {
        locked_catalog_descriptor_from_mock("bash")
    }

    fn package_with_same_outputs_as_outputs_to_install()
    -> LockedDescriptor<PackageDescriptorCatalog, LockedPackageCatalog> {
        locked_catalog_descriptor_from_mock("hello")
    }

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

    #[tokio::test(flavor = "multi_thread")]
    async fn v1_with_missing_lockfile_is_locked_before_migration() {
        let (mut flox, _tmpdir) = flox_instance();
        let env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        flox.features.outputs = true;
        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("envs/hello/hello.yaml")).await;

        std::fs::remove_file(env.lockfile_path(&flox).unwrap()).unwrap();

        let mut concrete = ConcreteEnvironment::Path(env);
        try_migrate_v1_to_v2(&flox, &mut concrete).unwrap();
        assert!(concrete.lockfile_path(&flox).unwrap().exists());
    }

    // #[test]
    // fn detects_writable_remote_env() {
    //     todo!()
    // }

    // #[test]
    // fn detects_writable_managed_env() {
    //     todo!()
    // }

    // #[test]
    // fn writable_v1_env_reported_as_migratable() {
    //     todo!()
    // }

    // #[test]
    // fn readonly_v1_env_reported_as_not_migratable() {
    //     todo!()
    // }

    // #[test]
    // fn writable_v2_env_reported_as_no_migration_needed() {
    //     todo!()
    // }

    // #[test]
    // fn readonly_v2_env_reported_as_no_migration_needed() {
    //     todo!()
    // }

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
        assert_eq!(collected.store_path.len(), 0);

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
    fn identifies_package_that_needs_migration() {
        let locked_pd = package_with_different_outputs_to_install();
        assert!(locked_pd.needs_migration());
    }

    #[test]
    fn identifies_package_that_doesnt_need_migration() {
        let locked_pd = package_with_same_outputs_as_outputs_to_install();
        assert!(!locked_pd.needs_migration());
    }

    // TODO: add tests for flake packages needing migration

    #[test]
    fn identifies_package_with_system_specific_outputs() {
        // This uses a locked Bash package, which contains a `debug` output
        // only available on Linux systems.
        let lockfile_path =
            CanonicalPath::new_unchecked(GENERATED_DATA.join("envs/bash/manifest.lock"));
        let lockfile = Lockfile::read_from_file(&lockfile_path).unwrap();
        let descriptor = lockfile.manifest.install.inner().get("bash").unwrap();

        let migrated = migrate_package_descriptor_v1_to_v2(descriptor, &lockfile);
        assert!(!migrated.ommitted_outputs.is_empty());
        let debug_output_systems = migrated.ommitted_outputs.get("debug").unwrap();
        assert_eq!(debug_output_systems, &vec![
            "aarch64-linux".to_string(),
            "x86_64-linux".to_string()
        ]);
    }

    #[test]
    fn identifies_package_with_no_system_specific_outputs() {
        // This uses a locked Bash package, which contains a `debug` output
        // only available on Linux systems.
        let lockfile_path =
            CanonicalPath::new_unchecked(GENERATED_DATA.join("envs/hello/manifest.lock"));
        let lockfile = Lockfile::read_from_file(&lockfile_path).unwrap();
        let descriptor = lockfile.manifest.install.inner().get("hello").unwrap();

        let migrated = migrate_package_descriptor_v1_to_v2(descriptor, &lockfile);
        assert!(migrated.ommitted_outputs.is_empty());
    }

    // #[test]
    // fn migrated_package_contains_all_outputs() {
    //     todo!()
    // }

    // #[test]
    // fn package_not_needing_migration_is_untouched() {
    //     todo!()
    // }

    // #[test]
    // fn migration_updates_manifest_version() {
    //     todo!()
    // }

    // #[test]
    // fn can_migrate_local_environment() {
    //     todo!()
    // }

    // #[test]
    // fn can_migrate_remote_environment() {
    //     todo!()
    // }

    // #[test]
    // fn can_migrate_managed_environment() {
    //     todo!()
    // }

    // #[test]
    // fn migration_creates_new_generation_for_floxhub_env() {
    //     todo!()
    // }
}
