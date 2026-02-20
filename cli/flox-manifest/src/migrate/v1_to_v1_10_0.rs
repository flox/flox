use std::collections::{HashMap, HashSet};

use flox_core::data::System;

use crate::lockfile::{
    LockedPackage,
    LockedPackageCatalog,
    LockedPackageFlake,
    Lockfile,
    PackageOutputs,
};
use crate::migrate::MigrationError;
use crate::parsed::v1::ManifestV1;
use crate::parsed::v1_10_0::{self, ManifestV1_10_0, SetOutputs};
use crate::parsed::{Inner, latest, v1};

/// Migrate a v1 manifest to a v1.10.0 manifest, optionally using an existing
/// lockfile to only set `outputs` on package descriptors that need it.
///
/// When the lockfile is missing or is stale and missing locked package data
/// for a package, the migrated package descriptor will unconditionally have its
/// `outputs` set to `"all"`. This is the only way to break a dependency cycle
/// between needing a lockfile to migrate, but needing a manifest in order to
/// create a lockfile.
pub(crate) fn migrate_manifest_v1_to_v1_10_0(
    manifest: &ManifestV1,
    lockfile: Option<&Lockfile>,
) -> Result<ManifestV1_10_0, MigrationError> {
    let mut migrated = ManifestV1_10_0 {
        schema_version: "1.10.0".to_string(),
        minimum_cli_version: Default::default(),
        install: latest::Install::default(),
        vars: manifest.vars.clone(),
        hook: manifest.hook.clone(),
        profile: manifest.profile.clone(),
        options: manifest.options.clone(),
        services: manifest.services.clone(),
        build: manifest.build.clone(),
        containerize: manifest.containerize.clone(),
        include: manifest.include.clone(),
    };

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
    // Note: We don't need to migrate store path packages, they just get passed through
    for (id, pd) in manifest.install.inner().iter().filter_map(|(id, mpd)| {
        mpd.as_store_path_descriptor_ref()
            .map(|store_path_pd| (id.clone(), store_path_pd))
    }) {
        install.insert(id, latest::ManifestPackageDescriptor::StorePath(pd.clone()));
    }

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
    /// The install ID of the package to migrate.
    install_id: String,
    /// The concrete packages descriptor kind (catalog or flake).
    pd: D,
    /// A map of system to `LockedPackageDescriptor*`, where this locked
    /// descriptor type must match that of the unlocked package descriptor
    /// type e.g. `PackageDescriptorCatalog` -> `LockedPackageCatalog`.
    ///
    /// If we don't have any locked package information for this package
    /// descriptor, this map will be empty and `outputs` will be set to
    /// "all". This is necessary in cases where we have a lockfile that's
    /// missing or stale.
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
        let all_outputs = self.output_intersection();
        let outputs_to_install = self.outputs_to_install_intersection();
        all_outputs != outputs_to_install
    }

    /// Returns the set of outputs that are available on all systems.
    fn output_intersection(&self) -> HashSet<OutputName>;

    /// Returns a deduplicated set of outputs to install.
    ///
    /// There's currently a bug in catalog-server that lists certain output
    /// names more than once. While this doesn't have any functional effects,
    /// when we go to compare the full list of outputs to the list of outputs
    /// to install, it will cause issues.
    fn outputs_to_install_intersection(&self) -> HashSet<OutputName>;

    /// Returns the migrated package descriptor.
    ///
    /// This may be a no-op for certain packages (store path packages and
    /// packages where `outputs_to_install` matches the list of all outputs).
    fn migrated(&self) -> latest::ManifestPackageDescriptor;
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
    P: Into<v1::ManifestPackageDescriptor> + Clone,
    // The locked package type
    L: PackageOutputs,
{
    fn output_intersection(&self) -> HashSet<OutputName> {
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

    fn outputs_to_install_intersection(&self) -> HashSet<OutputName> {
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

    fn migrated(&self) -> latest::ManifestPackageDescriptor {
        let mut pd: v1_10_0::ManifestPackageDescriptor = self.pd.clone().into().into();
        if self.locked.is_empty() || self.needs_migration() {
            pd.set_outputs_to_all();
        }
        pd
    }
}

/// The pairings between concrete package descriptor types from the manifest,
/// and their locked variants as collected from the lockfile.
struct CollectedPackages {
    catalog: Vec<LockedDescriptor<v1::PackageDescriptorCatalog, LockedPackageCatalog>>,
    flake: Vec<LockedDescriptor<v1::PackageDescriptorFlake, LockedPackageFlake>>,
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

/// Partitions the locked packages based on whether they come from
/// the catalog, a flake, or a store path.
fn collect_locked_packages_by_kind(
    manifest: &ManifestV1,
    lockfile: Option<&Lockfile>,
) -> Result<CollectedPackages, MigrationError> {
    let mut catalog_pkgs = vec![];
    let mut flake_pkgs = vec![];
    if let Some(lockfile) = lockfile {
        for (install_id, descriptor) in manifest.install.inner().iter() {
            use v1::ManifestPackageDescriptor::*;
            match descriptor {
                Catalog(pd) => {
                    let locked = get_locked_packages_by_install_id(install_id, lockfile);
                    if locked.is_none() {
                        let locked_descriptor = LockedDescriptor {
                            install_id: install_id.to_string(),
                            pd: pd.clone(),
                            locked: HashMap::new(),
                        };
                        catalog_pkgs.push(locked_descriptor);
                        continue;
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
                    if locked.is_none() {
                        let locked_descriptor = LockedDescriptor {
                            install_id: install_id.to_string(),
                            pd: pd.clone(),
                            locked: HashMap::new(),
                        };
                        flake_pkgs.push(locked_descriptor);
                        continue;
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
                StorePath(_) => {}, // we don't migrate store path packages
            }
        }
    } else {
        for (install_id, descriptor) in manifest.install.inner().iter() {
            use v1::ManifestPackageDescriptor::*;
            match descriptor {
                Catalog(pd) => {
                    let locked_descriptor = LockedDescriptor {
                        install_id: install_id.clone(),
                        pd: pd.clone(),
                        locked: HashMap::new(),
                    };
                    catalog_pkgs.push(locked_descriptor);
                },
                FlakeRef(pd) => {
                    let locked_descriptor = LockedDescriptor {
                        install_id: install_id.clone(),
                        pd: pd.clone(),
                        locked: HashMap::new(),
                    };
                    flake_pkgs.push(locked_descriptor);
                },
                StorePath(_) => {}, // we don't migrate store path packages
            }
        }
    }

    let collected = CollectedPackages {
        catalog: catalog_pkgs,
        flake: flake_pkgs,
    };
    Ok(collected)
}

#[cfg(test)]
mod tests {
    use flox_core::canonical_path::CanonicalPath;
    use flox_test_utils::GENERATED_DATA;

    use super::*;
    use crate::Manifest;
    use crate::interfaces::{InnerManifest, SchemaVersion};
    use crate::lockfile::test_helpers::{
        locked_package_catalog_from_mock_all_systems,
        locked_package_flake_from_mock_all_systems,
    };
    use crate::parsed::common::KnownSchemaVersion;
    use crate::parsed::latest::SelectedOutputs;
    use crate::parsed::v1_10_0;

    fn locked_catalog_descriptor_from_mock(
        install_id: &str,
    ) -> LockedDescriptor<v1::PackageDescriptorCatalog, LockedPackageCatalog> {
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
    ) -> LockedDescriptor<v1::PackageDescriptorFlake, LockedPackageFlake> {
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
    -> LockedDescriptor<v1::PackageDescriptorCatalog, LockedPackageCatalog> {
        locked_catalog_descriptor_from_mock("bash")
    }

    fn catalog_package_with_same_outputs_as_outputs_to_install()
    -> LockedDescriptor<v1::PackageDescriptorCatalog, LockedPackageCatalog> {
        locked_catalog_descriptor_from_mock("hello")
    }

    fn flake_package_with_different_outputs_to_install()
    -> LockedDescriptor<v1::PackageDescriptorFlake, LockedPackageFlake> {
        locked_flake_descriptor_from_mock("bash")
    }

    fn flake_package_with_same_outputs_as_outputs_to_install()
    -> LockedDescriptor<v1::PackageDescriptorFlake, LockedPackageFlake> {
        locked_flake_descriptor_from_mock("hello")
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
        let manifest: v1::ManifestV1 = Manifest::parse_toml_typed(&contents)
            .unwrap()
            .inner_manifest::<ManifestV1>()
            .unwrap()
            .clone();
        let lockfile_path =
            CanonicalPath::new_unchecked(GENERATED_DATA.join("envs/krb5_prereqs/manifest.lock"));
        let lockfile = Lockfile::read_from_file(&lockfile_path).unwrap();

        let collected = collect_locked_packages_by_kind(&manifest, Some(&lockfile)).unwrap();

        assert_eq!(collected.catalog.len(), 7);
        assert_eq!(collected.flake.len(), 0);

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
        let v1_10_0::ManifestPackageDescriptor::Catalog(pd) = migrated else {
            panic!("expected catalog package");
        };
        assert_eq!(pd.outputs, Some(SelectedOutputs::all()));
    }

    #[test]
    fn package_not_needing_migration_is_untouched() {
        let locked_descriptor = catalog_package_with_same_outputs_as_outputs_to_install();
        let migrated = locked_descriptor.migrated();
        let v1_10_0::ManifestPackageDescriptor::Catalog(pd) = migrated else {
            panic!("expected catalog package");
        };
        assert_eq!(pd.outputs, None);
    }

    #[test]
    fn migration_updates_manifest_version() {
        let manifest_path = GENERATED_DATA.join("envs/krb5_prereqs/manifest.toml");
        let contents = std::fs::read_to_string(manifest_path).unwrap();
        let manifest = Manifest::parse_toml_typed(contents)
            .unwrap()
            .inner_manifest::<ManifestV1>()
            .unwrap()
            .clone();
        let lockfile_path =
            CanonicalPath::new_unchecked(GENERATED_DATA.join("envs/krb5_prereqs/manifest.lock"));
        let lockfile = Lockfile::read_from_file(&lockfile_path).unwrap();

        let migrated = migrate_manifest_v1_to_v1_10_0(&manifest, Some(&lockfile)).unwrap();
        assert_eq!(migrated.get_schema_version(), KnownSchemaVersion::V1_10_0);
    }
}
