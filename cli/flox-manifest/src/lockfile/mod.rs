use std::sync::LazyLock;

mod catalog;
mod compose;
mod flake;
mod store_path;

use flox_core::data::{CanonicalPath, System};
use itertools::Itertools;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type FlakeRef = Value;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::fs;
use std::str::FromStr;

use flox_core::Version;

use crate::Manifest;
use crate::lockfile::catalog::LockedPackageCatalog;
use crate::lockfile::compose::Compose;
use crate::lockfile::flake::LockedPackageFlake;
use crate::lockfile::store_path::LockedPackageStorePath;

#[derive(Debug, thiserror::Error)]
pub enum LockfileError {
    #[error("failed to parse lockfile JSON: {0}")]
    ParseJson(#[source] serde_json::Error),

    #[error("failed to read lockfile: {0}")]
    IORead(#[source] std::io::Error),

    #[error("failed to write lockfile: {0}")]
    IOWrite(#[source] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Input {
    pub from: FlakeRef,
    #[serde(flatten)]
    _json: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    pub inputs: BTreeMap<String, Input>,
    #[serde(flatten)]
    _json: Value,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Lockfile {
    #[serde(rename = "lockfile-version")]
    pub version: Version<1>,
    /// The manifest that was locked.
    ///
    /// For an environment that doesn't include any others, this is the `manifest.toml`
    /// on disk at lock-time. For an environment that *does* include others, this is
    /// the merged manifest that was locked.
    pub manifest: Manifest,
    /// Locked packages
    pub packages: Vec<LockedPackage>,
    /// Composition information. This will be `None` when there are no includes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose: Option<Compose>, // use `is_none()` to detect composition
}

impl Lockfile {
    pub fn read_from_file(path: &CanonicalPath) -> Result<Self, LockfileError> {
        let contents = fs::read(path).map_err(LockfileError::IORead)?;
        serde_json::from_slice(&contents).map_err(LockfileError::ParseJson)
    }

    pub fn version(&self) -> u8 {
        1
    }
}

impl FromStr for Lockfile {
    type Err = LockfileError;

    fn from_str(contents: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(contents).map_err(LockfileError::ParseJson)
    }
}

impl Display for Lockfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", serde_json::json!(self))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, derive_more::From, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(untagged)]
pub enum LockedPackage {
    Catalog(LockedPackageCatalog),
    Flake(LockedPackageFlake),
    StorePath(LockedPackageStorePath),
}

impl LockedPackage {
    pub(crate) fn as_catalog_package_ref(&self) -> Option<&LockedPackageCatalog> {
        match self {
            LockedPackage::Catalog(pkg) => Some(pkg),
            _ => None,
        }
    }

    pub fn install_id(&self) -> &str {
        match self {
            LockedPackage::Catalog(pkg) => &pkg.install_id,
            LockedPackage::Flake(pkg) => &pkg.install_id,
            LockedPackage::StorePath(pkg) => &pkg.install_id,
        }
    }

    pub(crate) fn system(&self) -> &System {
        match self {
            LockedPackage::Catalog(pkg) => &pkg.system,
            LockedPackage::Flake(pkg) => &pkg.locked_installable.system,
            LockedPackage::StorePath(pkg) => &pkg.system,
        }
    }

    pub fn broken(&self) -> Option<bool> {
        match self {
            LockedPackage::Catalog(pkg) => pkg.broken,
            LockedPackage::Flake(pkg) => pkg.locked_installable.broken,
            LockedPackage::StorePath(_) => None,
        }
    }

    pub fn unfree(&self) -> Option<bool> {
        match self {
            LockedPackage::Catalog(pkg) => pkg.unfree,
            LockedPackage::Flake(pkg) => pkg.locked_installable.unfree,
            LockedPackage::StorePath(_) => None,
        }
    }

    pub fn derivation(&self) -> Option<&str> {
        match self {
            LockedPackage::Catalog(pkg) => Some(&pkg.derivation),
            LockedPackage::Flake(pkg) => Some(&pkg.locked_installable.derivation),
            // Technically store paths _may_ have a derivation,
            // but it's not quite relevant yet for us to record it in the lockfile.
            LockedPackage::StorePath(_) => None,
        }
    }

    pub fn version(&self) -> Option<&str> {
        match self {
            LockedPackage::Catalog(pkg) => Some(&pkg.version),
            LockedPackage::Flake(pkg) => pkg.locked_installable.version.as_deref(),
            LockedPackage::StorePath(_) => None,
        }
    }

    /// Convert a locked manifest to a list of installed packages for a given system.
    pub fn list_packages(&self, system: &System) -> Result<Vec<PackageToList>, ResolveError> {
        self.packages
            .iter()
            .filter(|package| package.system() == system)
            .cloned()
            .map(|package| match package {
                LockedPackage::Catalog(pkg) => {
                    let descriptor = self
                        .manifest
                        .pkg_descriptor_with_id(&pkg.install_id)
                        .ok_or(ResolveError::MissingPackageDescriptor(
                            pkg.install_id.clone(),
                        ))?;

                    let Some(descriptor) = descriptor.unwrap_catalog_descriptor() else {
                        Err(ResolveError::MissingPackageDescriptor(
                            pkg.install_id.clone(),
                        ))?
                    };

                    Ok(PackageToList::Catalog(descriptor, pkg))
                },
                LockedPackage::Flake(locked_package) => {
                    let descriptor = self
                        .manifest
                        .pkg_descriptor_with_id(&locked_package.install_id)
                        .ok_or(ResolveError::MissingPackageDescriptor(
                            locked_package.install_id.clone(),
                        ))?;

                    let Some(descriptor) = descriptor.unwrap_flake_descriptor() else {
                        Err(ResolveError::MissingPackageDescriptor(
                            locked_package.install_id.clone(),
                        ))?
                    };

                    Ok(PackageToList::Flake(descriptor, locked_package))
                },
                LockedPackage::StorePath(locked) => Ok(PackageToList::StorePath(locked)),
            })
            .collect::<Result<Vec<_>, ResolveError>>()
    }

    /// The manifest the user edits (i.e. not merged)
    pub fn user_manifest(&self) -> &Manifest {
        match &self.compose {
            Some(compose) => &compose.composer,
            None => &self.manifest,
        }
    }

    /// Transform a lockfile into a mapping that is easier to query:
    /// Lockfile -> { (install_id, system): (package_descriptor, locked_package) }
    fn make_seed_mapping(
        seed: &Lockfile,
    ) -> HashMap<(&str, &str), (&ManifestPackageDescriptor, &LockedPackage)> {
        seed.packages
            .iter()
            .filter_map(|locked| {
                let system = locked.system().as_str();
                let install_id = locked.install_id();
                let descriptor = seed.manifest.install.inner().get(locked.install_id())?;
                Some(((install_id, system), (descriptor, locked)))
            })
            .collect()
    }

    /// Creates package groups from a flat map of (catalog) install descriptors
    ///
    /// A group is created for each unique combination of (`descriptor.package_group` ｘ `descriptor.systems``).
    /// If descriptor.systems is [None], a group with `default_system` is created for each `package_group`.
    /// Each group contains a list of package descriptors that belong to that group.
    ///
    /// `seed_lockfile` is used to provide existing derivations for packages that are already locked,
    /// e.g. by a previous lockfile.
    /// These packages are used to constrain the resolution.
    /// If a package in `manifest` does not have a corresponding package in `seed_lockfile`,
    /// that package will be unconstrained, allowing a first install.
    ///
    /// As package groups only apply to catalog descriptors,
    /// this function **ignores other [ManifestPackageDescriptor] variants**.
    /// Those are expected to be locked separately.
    ///
    /// Greenkeeping: this function seem to return a [Result]
    /// only due to parsing [System] strings to [PackageSystem].
    /// If we restricted systems earlier with a common `System` type,
    /// fallible conversions like that would be unnecessary,
    /// or would be pushed higher up.
    fn collect_package_groups(
        manifest: &Manifest,
        seed_lockfile: Option<&Lockfile>,
    ) -> Result<impl Iterator<Item = PackageGroup>, ResolveError> {
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        // Using a btree map to ensure consistent ordering
        let mut map = BTreeMap::new();

        let manifest_systems = manifest.options.systems.as_deref();

        let maybe_licenses = manifest
            .options
            .allow
            .licenses
            .clone()
            .and_then(|licenses| {
                if licenses.is_empty() {
                    None
                } else {
                    Some(licenses)
                }
            });

        for (install_id, manifest_descriptor) in manifest.install.inner().iter() {
            // package groups are only relevant to catalog descriptors
            let Some(manifest_descriptor) = manifest_descriptor.as_catalog_descriptor_ref() else {
                continue;
            };

            let resolved_descriptor_base = PackageDescriptor {
                install_id: install_id.clone(),
                attr_path: manifest_descriptor.pkg_path.clone(),
                derivation: None,
                version: manifest_descriptor.version.clone(),
                allow_pre_releases: manifest.options.semver.allow_pre_releases,
                allow_broken: manifest.options.allow.broken,
                // TODO: add support for insecure
                allow_insecure: None,
                allow_unfree: manifest.options.allow.unfree,
                allow_missing_builds: None,
                allowed_licenses: maybe_licenses.clone(),
                systems: vec![],
            };

            let group_name = manifest_descriptor
                .pkg_group
                .as_deref()
                .unwrap_or(DEFAULT_GROUP_NAME);

            let resolved_group =
                map.entry(group_name.to_string())
                    .or_insert_with(|| PackageGroup {
                        descriptors: Vec::new(),
                        name: group_name.to_string(),
                    });

            let systems = {
                let available_systems = manifest_systems.unwrap_or(&*DEFAULT_SYSTEMS_STR);

                let package_systems = manifest_descriptor.systems.as_deref();

                for system in package_systems.into_iter().flatten() {
                    if !available_systems.contains(system) {
                        return Err(ResolveError::SystemUnavailableInManifest {
                            install_id: install_id.clone(),
                            system: system.to_string(),
                            enabled_systems: available_systems
                                .iter()
                                .map(|s| s.to_string())
                                .collect(),
                        });
                    }
                }

                package_systems
                    .or(manifest_systems)
                    .unwrap_or(&*DEFAULT_SYSTEMS_STR)
                    .iter()
                    .sorted()
                    .map(|s| {
                        PackageSystem::from_str(s)
                            .map_err(|_| ResolveError::UnrecognizedSystem(s.to_string()))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };

            for system in systems {
                // If the package was just added to the manifest, it will be missing in the seed,
                // which is derived from the _previous_ lockfile.
                // In this case, the derivation will be None, and the package will be unconstrained.
                //
                // If the package was already locked, but the descriptor has changed in a way
                // that invalidates the existing resolution, the derivation will be None.
                //
                // If the package was locked from a flake installable before
                // it needs to be re-resolved with the catalog, so the derivation will be None.
                let locked_derivation = seed_locked_packages
                    .get(&(install_id, &system.to_string()))
                    .filter(|(descriptor, _)| {
                        !descriptor.invalidates_existing_resolution(&manifest_descriptor.into())
                    })
                    .and_then(|(_, locked_package)| locked_package.as_catalog_package_ref())
                    .map(|locked_package| locked_package.derivation.clone());

                let mut resolved_descriptor = resolved_descriptor_base.clone();

                resolved_descriptor.systems = vec![system];
                resolved_descriptor.derivation = locked_derivation;

                resolved_group.descriptors.push(resolved_descriptor);
            }
        }
        Ok(map.into_values())
    }
}

/// Distinct types of packages that can be listed
/// TODO: drop in favor of mapping to `(ManifestPackageDescriptor*, LockedPackage*)`
#[derive(Debug, Clone, PartialEq)]
pub enum PackageToList {
    Catalog(PackageDescriptorCatalog, LockedPackageCatalog),
    Flake(PackageDescriptorFlake, LockedPackageFlake),
    StorePath(LockedPackageStorePath),
}

pub mod test_helpers {
    use super::*;
    use crate::models::manifest::typed::PackageDescriptorStorePath;

    pub fn fake_catalog_package_lock(
        name: &str,
        group: Option<&str>,
    ) -> (String, ManifestPackageDescriptor, LockedPackageCatalog) {
        let install_id = format!("{}_install_id", name);

        let descriptor = PackageDescriptorCatalog {
            pkg_path: name.to_string(),
            pkg_group: group.map(|s| s.to_string()),
            systems: Some(vec![PackageSystem::Aarch64Darwin.to_string()]),
            version: None,
            priority: None,
            outputs: None,
        }
        .into();

        let locked = LockedPackageCatalog {
            attr_path: name.to_string(),
            broken: None,
            derivation: "derivation".to_string(),
            description: None,
            install_id: install_id.clone(),
            license: None,
            locked_url: "".to_string(),
            name: name.to_string(),
            outputs: Default::default(),
            outputs_to_install: None,
            pname: name.to_string(),
            rev: "".to_string(),
            rev_count: 0,
            rev_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::offset::Utc),
            scrape_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::offset::Utc),
            stabilities: None,
            unfree: None,
            version: "".to_string(),
            system: PackageSystem::Aarch64Darwin.to_string(),
            group: group.unwrap_or(DEFAULT_GROUP_NAME).to_string(),
            priority: 5,
        };
        (install_id, descriptor, locked)
    }

    pub fn fake_flake_installable_lock(
        name: &str,
    ) -> (String, PackageDescriptorFlake, LockedPackageFlake) {
        let install_id = format!("{}_install_id", name);

        let descriptor = PackageDescriptorFlake {
            flake: format!("github:nowhere/exciting#{name}"),
            priority: None,
            systems: None,
            outputs: None,
        };

        let locked = LockedPackageFlake {
            install_id: install_id.clone(),
            locked_installable: LockedInstallable {
                locked_url: format!(
                    "github:nowhere/exciting/affeaffeaffeaffeaffeaffeaffeaffeaffeaffe#{name}"
                ),
                flake_description: None,
                locked_flake_attr_path: format!("packages.aarch64-darwin.{name}"),
                derivation: "derivation".to_string(),
                outputs: Default::default(),
                output_names: vec![],
                outputs_to_install: None,
                requested_outputs_to_install: None,
                package_system: "aarch64-darwin".to_string(),
                system: "aarch64-darwin".to_string(),
                name: format!("{name}-1.0.0"),
                pname: Some(name.to_string()),
                version: Some("1.0.0".to_string()),
                description: None,
                licenses: None,
                broken: None,
                unfree: None,
                priority: DEFAULT_PRIORITY,
            },
        };
        (install_id, descriptor, locked)
    }

    pub fn fake_store_path_lock(
        name: &str,
    ) -> (String, PackageDescriptorStorePath, LockedPackageStorePath) {
        let install_id = format!("{}_install_id", name);

        let descriptor = PackageDescriptorStorePath {
            store_path: format!("/nix/store/{}", name),
            systems: Some(vec![PackageSystem::Aarch64Darwin.to_string()]),
            priority: None,
        };

        let locked = LockedPackageStorePath {
            install_id: install_id.clone(),
            store_path: format!("/nix/store/{}", name),
            system: PackageSystem::Aarch64Darwin.to_string(),
            priority: DEFAULT_PRIORITY,
        };
        (install_id, descriptor, locked)
    }

    pub fn nix_eval_jobs_descriptor() -> PackageDescriptorFlake {
        PackageDescriptorFlake {
            flake: "github:nix-community/nix-eval-jobs".to_string(),
            priority: None,
            systems: None,
            outputs: None,
        }
    }

    /// This JSON was copied from a manifest.lock after installing github:nix-community/nix-eval-jobs
    pub static LOCKED_NIX_EVAL_JOBS: LazyLock<LockedPackageFlake> = LazyLock::new(|| {
        serde_json::from_str(r#"
            {
              "install_id": "nix-eval-jobs",
              "locked-url": "github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b",
              "flake-description": "Hydra's builtin hydra-eval-jobs as a standalone",
              "locked-flake-attr-path": "packages.aarch64-darwin.default",
              "derivation": "/nix/store/29y1mrdqncdjfkdfa777zmspp9djzb6b-nix-eval-jobs-2.23.0.drv",
              "outputs": {
                "out": "/nix/store/qigv8kbk1gpk0g2pfw10lbmdy44cf06r-nix-eval-jobs-2.23.0"
              },
              "output-names": [
                "out"
              ],
              "outputs-to-install": [
                "out"
              ],
              "package-system": "aarch64-darwin",
              "system": "aarch64-darwin",
              "name": "nix-eval-jobs-2.23.0",
              "pname": "nix-eval-jobs",
              "version": "2.23.0",
              "description": "Hydra's builtin hydra-eval-jobs as a standalone",
              "licenses": [
                "GPL-3.0"
              ],
              "broken": false,
              "unfree": false
            }
        "#).unwrap()
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::LazyLock;
    use std::vec;

    use catalog::MsgUnknown;
    use catalog_api_v1::types::{PackageOutput, PackageOutputs};
    use indoc::indoc;
    use pollster::FutureExt;
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use test_helpers::{
        fake_catalog_package_lock,
        fake_flake_installable_lock,
        fake_store_path_lock,
    };

    use self::catalog::{CatalogPage, PackageResolutionInfo};
    use super::*;
    use crate::flox::RemoteEnvironmentRef;
    use crate::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use crate::models::environment::Environment;
    use crate::models::environment::fetcher::test_helpers::mock_include_fetcher;
    use crate::models::environment::path_environment::test_helpers::{
        new_named_path_environment_in,
        new_path_environment,
        new_path_environment_in,
    };
    use crate::models::environment::path_environment::tests::generate_path_environments_without_install_or_include;
    use crate::models::environment::remote_environment::test_helpers::mock_remote_environment;
    use crate::models::manifest::raw::RawManifest;
    use crate::models::manifest::typed::{Include, Manifest, Vars};
    use crate::providers::catalog::test_helpers::{
        auto_recording_catalog_client,
        catalog_replay_client,
    };
    use crate::providers::catalog::{GENERATED_DATA, MockClient};
    use crate::providers::flake_installable_locker::{
        FlakeInstallableError,
        InstallableLockerMock,
    };

    struct PanickingLocker;
    impl InstallableLocker for PanickingLocker {
        fn lock_flake_installable(
            &self,
            _: impl AsRef<str>,
            _: &PackageDescriptorFlake,
        ) -> Result<LockedInstallable, FlakeInstallableError> {
            panic!("this flake locker always panics")
        }
    }

    static TEST_RAW_MANIFEST: LazyLock<RawManifest> = LazyLock::new(|| {
        indoc! {r#"
          version = 1

          [install]
          hello_install_id.pkg-path = "hello"
          hello_install_id.pkg-group = "group"

          [options]
          systems = ["aarch64-darwin"]
        "#}
        .parse()
        .unwrap()
    });

    static TEST_TYPED_MANIFEST: LazyLock<Manifest> =
        LazyLock::new(|| TEST_RAW_MANIFEST.to_typed().unwrap());

    static TEST_RESOLUTION_PARAMS: LazyLock<Vec<PackageGroup>> = LazyLock::new(|| {
        vec![PackageGroup {
            name: "group".to_string(),
            descriptors: vec![PackageDescriptor {
                install_id: "hello_install_id".to_string(),
                attr_path: "hello".to_string(),
                derivation: None,
                version: None,
                allow_pre_releases: None,
                allow_broken: None,
                allow_insecure: None,
                allow_unfree: None,
                allowed_licenses: None,
                allow_missing_builds: None,
                systems: vec![PackageSystem::Aarch64Darwin],
            }],
        }]
    });

    static TEST_LOCKED_MANIFEST: LazyLock<Lockfile> = LazyLock::new(|| Lockfile {
        version: Version::<1>,
        manifest: TEST_TYPED_MANIFEST.clone(),
        packages: vec![
            LockedPackageCatalog {
                attr_path: "hello".to_string(),
                broken: Some(false),
                derivation: "derivation".to_string(),
                description: Some("description".to_string()),
                install_id: "hello_install_id".to_string(),
                license: Some("license".to_string()),
                locked_url: "locked_url".to_string(),
                name: "hello".to_string(),
                outputs: [("name".to_string(), "store_path".to_string())]
                    .into_iter()
                    .collect(),

                outputs_to_install: Some(vec!["name".to_string()]),
                pname: "pname".to_string(),
                rev: "rev".to_string(),
                rev_count: 1,
                rev_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::offset::Utc),
                scrape_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::offset::Utc),
                stabilities: Some(vec!["stability".to_string()]),
                unfree: Some(false),
                version: "version".to_string(),
                system: PackageSystem::Aarch64Darwin.to_string(),
                group: "group".to_string(),
                priority: 5,
            }
            .into(),
        ],
        compose: None,
    });

    #[test]
    fn make_params_smoke() {
        let manifest = &*TEST_TYPED_MANIFEST;

        let params = Lockfile::collect_package_groups(manifest, None)
            .unwrap()
            .collect::<Vec<_>>();
        assert_eq!(&params, &*TEST_RESOLUTION_PARAMS);
    }

    /// When `options.systems` defines multiple systems,
    /// request groups for each system separately.
    #[test]
    fn make_params_multiple_systems() {
        let manifest_str = indoc! {r#"
            version = 1

            [install]
            vim.pkg-path = "vim"
            emacs.pkg-path = "emacs"

            [options]
            systems = ["aarch64-darwin", "x86_64-linux"]
        "#};
        let manifest = toml::from_str(manifest_str).unwrap();

        let expected_params = vec![PackageGroup {
            name: DEFAULT_GROUP_NAME.to_string(),
            descriptors: vec![
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "emacs".to_string(),
                    derivation: None,
                    install_id: "emacs".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "emacs".to_string(),
                    derivation: None,
                    install_id: "emacs".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::X8664Linux],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::X8664Linux],
                },
            ],
        }];

        let actual_params = Lockfile::collect_package_groups(&manifest, None)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(actual_params, expected_params);
    }

    /// When `options.systems` defines multiple systems,
    /// request groups for each system separately.
    /// If a package specifies systems, use those instead.
    #[test]
    fn make_params_limit_systems() {
        let manifest_str = indoc! {r#"
            version = 1

            [install]
            vim.pkg-path = "vim"
            emacs.pkg-path = "emacs"
            emacs.systems = ["aarch64-darwin" ]

            [options]
            systems = ["aarch64-darwin", "x86_64-linux"]
        "#};
        let manifest = toml::from_str(manifest_str).unwrap();

        let expected_params = vec![PackageGroup {
            name: DEFAULT_GROUP_NAME.to_string(),
            descriptors: vec![
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "emacs".to_string(),
                    install_id: "emacs".to_string(),
                    derivation: None,
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::X8664Linux],
                },
            ],
        }];

        let actual_params = Lockfile::collect_package_groups(&manifest, None)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(actual_params, expected_params);
    }

    /// If a package specifies a system not in `options.systems`,
    /// return an error.
    #[test]
    fn descriptor_system_required_in_options() {
        let manifest_str = indoc! {r#"
            version = 1

            [install]
            vim.pkg-path = "vim"
            emacs.pkg-path = "emacs"
            emacs.systems = ["aarch64-darwin" ]

            [options]
            systems = ["x86_64-linux"]
        "#};

        // todo: ideally the manifest would not even parse if it has an unavailable system
        let manifest = toml::from_str(manifest_str).unwrap();

        let actual_result = Lockfile::collect_package_groups(&manifest, None);

        assert!(
            matches!(actual_result, Err(ResolveError::SystemUnavailableInManifest {
                install_id,
                system,
                enabled_systems
            }) if install_id == "emacs" && system == "aarch64-darwin" && enabled_systems == vec!["x86_64-linux"])
        );
    }

    /// If packages specify different groups,
    /// create request groups for each group.
    #[test]
    fn make_params_groups() {
        let manifest_str = indoc! {r#"
            version = 1

            [install]
            vim.pkg-path = "vim"
            vim.pkg-group = "group1"

            emacs.pkg-path = "emacs"
            emacs.pkg-group = "group2"

            [options]
            systems = ["aarch64-darwin"]
        "#};

        let manifest = toml::from_str(manifest_str).unwrap();

        let expected_params = vec![
            PackageGroup {
                name: "group1".to_string(),
                descriptors: vec![PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                }],
            },
            PackageGroup {
                name: "group2".to_string(),
                descriptors: vec![PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "emacs".to_string(),
                    derivation: None,
                    install_id: "emacs".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                }],
            },
        ];

        let actual_params = Lockfile::collect_package_groups(&manifest, None)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(actual_params, expected_params);
    }

    /// If a seed mapping is provided, use the derivations from the seed where possible
    #[test]
    fn make_params_seeded() {
        let mut manifest = TEST_TYPED_MANIFEST.clone();

        // Add a package to the manifest that is not already locked
        manifest.install.inner_mut().insert(
            "unlocked".to_string(),
            PackageDescriptorCatalog {
                pkg_path: "unlocked".to_string(),
                pkg_group: Some("group".to_string()),
                systems: None,
                version: None,
                priority: None,
                outputs: None,
            }
            .into(),
        );

        let actual_params =
            Lockfile::collect_package_groups(&manifest, Some(&*TEST_LOCKED_MANIFEST))
                .unwrap()
                .collect::<Vec<_>>();

        let expected_params = vec![PackageGroup {
            name: "group".to_string(),
            descriptors: vec![
                // 'hello' was already locked, so it should have a derivation
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "hello".to_string(),
                    derivation: Some("derivation".to_string()),
                    install_id: "hello_install_id".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                },
                // The unlocked package should not have a derivation
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "unlocked".to_string(),
                    derivation: None,
                    install_id: "unlocked".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin],
                },
            ],
        }];

        assert_eq!(actual_params, expected_params);
    }

    /// If a seed mapping is provided, use the derivations from the seed where possible
    /// 1) If the package is unchanged, it should not be re-resolved.
    #[test]
    fn make_params_seeded_unchanged() {
        let (foo_before_iid, foo_before_descriptor, foo_before_locked) =
            fake_catalog_package_lock("foo", None);
        let mut manifest_before = Manifest::default();
        manifest_before
            .install
            .inner_mut()
            .insert(foo_before_iid.clone(), foo_before_descriptor.clone());

        let seed = Lockfile {
            version: Version::<1>,
            manifest: manifest_before.clone(),
            packages: vec![foo_before_locked.clone().into()],
            compose: None,
        };

        // ---------------------------------------------------------------------

        let actual_params = Lockfile::collect_package_groups(&manifest_before, Some(&seed))
            .unwrap()
            .collect::<Vec<_>>();

        // the original derivation should be present and unchanged
        assert_eq!(
            actual_params[0].descriptors[0].derivation.as_ref(),
            Some(&foo_before_locked.derivation)
        );
    }

    /// If a seed mapping is provided, use the derivations from the seed where possible
    /// 2) Changes that invalidate the locked package should cause it to be re-resolved.
    ///    Here, the package path is changed.
    #[test]
    fn make_params_seeded_unlock_if_invalidated() {
        let (foo_before_iid, foo_before_descriptor, foo_before_locked) =
            fake_catalog_package_lock("foo", None);
        let mut manifest_before = Manifest::default();
        manifest_before
            .install
            .inner_mut()
            .insert(foo_before_iid.clone(), foo_before_descriptor.clone());

        let seed = Lockfile {
            version: Version::<1>,
            manifest: manifest_before.clone(),
            packages: vec![foo_before_locked.into()],
            compose: None,
        };

        // ---------------------------------------------------------------------

        let (foo_after_iid, mut foo_after_descriptor, _) = fake_catalog_package_lock("foo", None);

        if let ManifestPackageDescriptor::Catalog(ref mut descriptor) = foo_after_descriptor {
            descriptor.pkg_path = "bar".to_string();
        } else {
            panic!("Expected a catalog descriptor");
        };

        assert!(foo_after_descriptor.invalidates_existing_resolution(&foo_before_descriptor));

        let mut manifest_after = Manifest::default();
        manifest_after
            .install
            .inner_mut()
            .insert(foo_after_iid.clone(), foo_after_descriptor.clone());

        let actual_params = Lockfile::collect_package_groups(&manifest_after, Some(&seed))
            .unwrap()
            .collect::<Vec<_>>();

        // if the package changed, it should be re-resolved
        // i.e. the derivation should be None
        assert_eq!(actual_params[0].descriptors[0].derivation.as_ref(), None);
    }

    /// If a seed mapping is provided, use the derivations from the seed where possible
    /// 3) Changes to the descriptor that do not invalidate the derivation
    ///    should not cause it to be re-resolved.
    ///    Here, the priority is changed.
    #[test]
    fn make_params_seeded_changed_no_invalidation() {
        let (foo_before_iid, foo_before_descriptor, foo_before_locked) =
            fake_catalog_package_lock("foo", None);
        let mut manifest_before = Manifest::default();
        manifest_before
            .install
            .inner_mut()
            .insert(foo_before_iid.clone(), foo_before_descriptor.clone());

        let seed = Lockfile {
            version: Version::<1>,
            manifest: manifest_before.clone(),
            packages: vec![foo_before_locked.clone().into()],
            compose: None,
        };

        // ---------------------------------------------------------------------

        let (foo_after_iid, mut foo_after_descriptor, _) = fake_catalog_package_lock("foo", None);
        if let ManifestPackageDescriptor::Catalog(ref mut descriptor) = foo_after_descriptor {
            descriptor.priority = Some(10);
        } else {
            panic!("Expected a catalog descriptor");
        };

        assert!(!foo_after_descriptor.invalidates_existing_resolution(&foo_before_descriptor));

        let mut manifest_after = Manifest::default();
        manifest_after
            .install
            .inner_mut()
            .insert(foo_after_iid.clone(), foo_after_descriptor.clone());

        let actual_params = Lockfile::collect_package_groups(&manifest_after, Some(&seed))
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(
            actual_params[0].descriptors[0].derivation.as_ref(),
            Some(&foo_before_locked.derivation)
        );
    }

    /// If flake installables and catalog packages are mixed,
    /// [Lockfile::collect_package_groups]
    /// should only return [PackageGroup]s for the catalog descriptors.
    #[test]
    fn make_params_filters_installables() {
        let manifest_str = indoc! {r#"
            version = 1

            [install]
            vim.pkg-path = "vim"
            emacs.flake = "github:nixos/nixpkgs#emacs"

            [options]
            systems = ["aarch64-darwin", "x86_64-linux"]
        "#};
        let manifest = toml::from_str(manifest_str).unwrap();

        let expected_params = vec![PackageGroup {
            name: DEFAULT_GROUP_NAME.to_string(),
            descriptors: [PackageSystem::Aarch64Darwin, PackageSystem::X8664Linux]
                .map(|system| {
                    [PackageDescriptor {
                        allow_pre_releases: None,
                        attr_path: "vim".to_string(),
                        derivation: None,
                        install_id: "vim".to_string(),
                        version: None,
                        allow_broken: None,
                        allow_insecure: None,
                        allow_unfree: None,
                        allowed_licenses: None,
                        allow_missing_builds: None,
                        systems: vec![system],
                    }]
                })
                .into_iter()
                .flatten()
                .collect(),
        }];

        let actual_params = Lockfile::collect_package_groups(&manifest, None)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(actual_params, expected_params);
    }

    /// [Lockfile::collect_package_groups] generates [FlakeInstallableToLock]
    /// for each default system.
    #[test]
    fn make_installables_to_lock_for_default_systems() {
        let mut manifest = Manifest::default();
        let (foo_install_id, foo_descriptor, _) = fake_flake_installable_lock("foo");

        manifest
            .install
            .inner_mut()
            .insert(foo_install_id.clone(), foo_descriptor.clone().into());

        let expected = DEFAULT_SYSTEMS_STR
            .clone()
            .map(|system| FlakeInstallableToLock {
                install_id: foo_install_id.clone(),
                descriptor: foo_descriptor.clone(),
                system: system.to_string(),
            });

        let actual: Vec<_> = Lockfile::collect_flake_installables(&manifest).collect();

        assert_eq!(actual, expected);
    }

    /// [Lockfile::collect_package_groups] generates [FlakeInstallableToLock]
    /// for each system in the manifest.
    #[test]
    fn make_installables_to_lock_for_manifest_systems() {
        let system = "aarch64-darwin";

        let mut manifest = Manifest::default();
        manifest.options.systems = Some(vec![system.to_string()]);

        let (foo_install_id, foo_descriptor, _) = fake_flake_installable_lock("foo");

        manifest
            .install
            .inner_mut()
            .insert(foo_install_id.clone(), foo_descriptor.clone().into());

        let expected = [FlakeInstallableToLock {
            install_id: foo_install_id.clone(),
            descriptor: foo_descriptor.clone(),
            system: system.to_string(),
        }];

        let actual: Vec<_> = Lockfile::collect_flake_installables(&manifest).collect();

        assert_eq!(actual, expected);
    }

    /// If flake installables and catalog packages are mixed,
    /// [Lockfile::collect_flake_installables]
    /// should only return [FlakeInstallableToLock] for the flake installables.
    #[test]
    fn make_installables_to_lock_filter_catalog() {
        let mut manifest = Manifest::default();
        let (foo_install_id, foo_descriptor, _) = fake_flake_installable_lock("foo");
        let (bar_install_id, bar_descriptor, _) = fake_catalog_package_lock("bar", None);

        manifest
            .install
            .inner_mut()
            .insert(foo_install_id.clone(), foo_descriptor.clone().into());
        manifest
            .install
            .inner_mut()
            .insert(bar_install_id.clone(), bar_descriptor.clone());

        let expected = DEFAULT_SYSTEMS_STR
            .clone()
            .map(|system| FlakeInstallableToLock {
                install_id: foo_install_id.clone(),
                descriptor: foo_descriptor.clone(),
                system: system.to_string(),
            });

        let actual: Vec<_> = Lockfile::collect_flake_installables(&manifest).collect();

        assert_eq!(actual, expected);
    }

    #[test]
    fn ungroup_response() {
        let groups = vec![ResolvedPackageGroup {
            page: Some(CatalogPage {
                page: 1,
                complete: true,
                url: "url".to_string(),
                packages: Some(vec![PackageResolutionInfo {
                    catalog: None,
                    attr_path: "hello".to_string(),
                    pkg_path: "hello".to_string(),
                    broken: Some(false),
                    derivation: "derivation".to_string(),
                    description: Some("description".to_string()),
                    insecure: Some(false),
                    install_id: "hello_install_id".to_string(),
                    license: Some("license".to_string()),
                    locked_url: "locked_url".to_string(),
                    name: "hello".to_string(),
                    outputs: PackageOutputs(vec![PackageOutput {
                        name: "name".to_string(),
                        store_path: "store_path".to_string(),
                    }]),
                    outputs_to_install: Some(vec!["name".to_string()]),
                    pname: "pname".to_string(),
                    rev: "rev".to_string(),
                    rev_count: 1,
                    rev_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                        .unwrap()
                        .with_timezone(&chrono::offset::Utc),
                    scrape_date: Some(
                        chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                            .unwrap()
                            .with_timezone(&chrono::offset::Utc),
                    ),
                    stabilities: Some(vec!["stability".to_string()]),
                    unfree: Some(false),
                    version: "version".to_string(),
                    system: PackageSystem::Aarch64Darwin,
                    cache_uri: None,
                    missing_builds: None,
                }]),
                msgs: vec![],
            }),
            name: "group".to_string(),
            msgs: vec![],
        }];

        let manifest = &*TEST_TYPED_MANIFEST;

        let locked_packages = Lockfile::locked_packages_from_resolution(manifest, groups.clone())
            .unwrap()
            .collect::<Vec<_>>();

        let descriptor = manifest
            .install
            .inner()
            .get(&groups[0].page.as_ref().unwrap().packages.as_ref().unwrap()[0].install_id)
            .and_then(ManifestPackageDescriptor::as_catalog_descriptor_ref)
            .expect("expected a catalog descriptor")
            .clone();

        assert_eq!(locked_packages.len(), 1);
        assert_eq!(
            locked_packages[0],
            LockedPackageCatalog::from_parts(
                groups[0].page.as_ref().unwrap().packages.as_ref().unwrap()[0].clone(),
                descriptor,
            )
        );
    }

    /// Unlocking by iid should remove only the package with that iid.
    /// Both catalog packages and flake installables should be removed.
    #[test]
    fn unlock_by_iid() {
        let mut manifest = Manifest::default();
        let (foo_iid, foo_descriptor, foo_locked) = fake_catalog_package_lock("foo", None);
        let (bar_iid, bar_descriptor, bar_locked) = fake_catalog_package_lock("bar", None);
        let (baz_iid, baz_descriptor, baz_locked) = fake_flake_installable_lock("baz");
        let (qux_iid, qux_descriptor, qux_locked) = fake_flake_installable_lock("qux");
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor);
        manifest
            .install
            .inner_mut()
            .insert(bar_iid.clone(), bar_descriptor);
        manifest
            .install
            .inner_mut()
            .insert(baz_iid.clone(), baz_descriptor.into());
        manifest
            .install
            .inner_mut()
            .insert(qux_iid.clone(), qux_descriptor.into());
        let mut lockfile = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![
                foo_locked.into(),
                bar_locked.clone().into(),
                baz_locked.into(),
                qux_locked.clone().into(),
            ],
            compose: None,
        };

        lockfile.unlock_packages_by_group_or_iid(&[&foo_iid, &baz_iid]);

        assert_eq!(lockfile.packages, vec![
            bar_locked.into(),
            qux_locked.into()
        ]);
    }

    /// Unlocking by group should remove all packages in that group
    #[test]
    fn unlock_by_group() {
        let mut manifest = Manifest::default();
        let (foo_iid, foo_descriptor, foo_locked) = fake_catalog_package_lock("foo", Some("group"));
        let (bar_iid, bar_descriptor, bar_locked) = fake_catalog_package_lock("bar", Some("group"));
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor);
        manifest
            .install
            .inner_mut()
            .insert(bar_iid.clone(), bar_descriptor);
        let mut lockfile = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into(), bar_locked.into()],
            compose: None,
        };

        lockfile.unlock_packages_by_group_or_iid(&["group"]);

        assert_eq!(lockfile.packages, vec![]);
    }

    /// If an unlocked iid is also used as a group, remove both the group
    /// and the package
    #[test]
    fn unlock_by_iid_and_group() {
        let mut manifest = Manifest::default();
        let (foo_iid, foo_descriptor, foo_locked) =
            fake_catalog_package_lock("foo", Some("foo_install_id"));
        let (bar_iid, bar_descriptor, bar_locked) =
            fake_catalog_package_lock("bar", Some("foo_install_id"));
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor);
        manifest
            .install
            .inner_mut()
            .insert(bar_iid.clone(), bar_descriptor);
        let mut lockfile = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into(), bar_locked.into()],
            compose: None,
        };

        lockfile.unlock_packages_by_group_or_iid(&[&foo_iid]);

        assert_eq!(lockfile.packages, vec![]);
    }

    #[test]
    fn unlock_by_iid_noop_if_already_unlocked() {
        let mut seed = TEST_LOCKED_MANIFEST.clone();

        // If the package is not in the seed, the lockfile should be unchanged
        let expected = seed.packages.clone();

        seed.unlock_packages_by_group_or_iid(&["not in here"]);

        assert_eq!(seed.packages, expected,);
    }

    #[tokio::test]
    async fn locking_unknown_message() {
        let manifest = Manifest::from_str(indoc! {r#"
                version = 1

                [install]
                ps.pkg-path = "darwin.ps"

                [options]
                systems = [ "x86_64-darwin", "aarch64-linux" ]
            "#})
        .unwrap();

        let client = catalog_replay_client(
            GENERATED_DATA.join("resolve/darwin_ps_incompatible_transform_error_to_unknown.yaml"),
        )
        .await;

        let locked_manifest =
            Lockfile::resolve_manifest(&manifest, None, &client, &InstallableLockerMock::new())
                .await;
        if let Err(ResolveError::ResolutionFailed(res_failures)) = locked_manifest {
            assert_eq!(res_failures.to_string(), format!("\n{}", "unknown message"));
            if let [ResolutionFailure::UnknownServiceMessage(MsgUnknown { msg, .. })] =
                res_failures.0.as_slice()
            {
                assert_eq!(msg, "unknown message");
            } else {
                panic!(
                    "expected a single UnknownServiceMessage, got {:?}",
                    res_failures.0.as_slice()
                );
            }
        } else {
            panic!("expected resolution failure, got {:?}", locked_manifest);
        }
    }

    #[tokio::test]
    async fn locking_general_message() {
        let manifest = Manifest::from_str(indoc! {r#"
                version = 1

                [install]
                ps.pkg-path = "darwin.ps"

                [options]
                systems = [ "x86_64-darwin", "aarch64-linux" ]
            "#})
        .unwrap();

        let client = catalog_replay_client(
            GENERATED_DATA.join("resolve/darwin_ps_incompatible_transform_error_to_general.yaml"),
        )
        .await;

        let locked_manifest =
            Lockfile::resolve_manifest(&manifest, None, &client, &InstallableLockerMock::new())
                .await;
        if let Err(ResolveError::ResolutionFailed(res_failures)) = locked_manifest {
            assert_eq!(res_failures.to_string(), format!("\n{}", "general message"));
            if let [ResolutionFailure::FallbackMessage { msg, .. }] = res_failures.0.as_slice() {
                assert_eq!(msg, "general message");
            } else {
                panic!(
                    "expected a single FallbackMessage, got {:?}",
                    res_failures.0.as_slice()
                );
            }
        } else {
            panic!("expected resolution failure, got {:?}", locked_manifest);
        }
    }

    /// Test the server generated message is passed through for systems not on same page
    #[tokio::test]
    async fn locking_message_is_passed_through_for_systems_not_on_same_page() {
        let manifest = Manifest::from_str(indoc! {r#"
                version = 1

                [install]
                ps.pkg-path = "darwin.ps"

                [options]
                systems = [ "x86_64-darwin", "aarch64-linux" ]
            "#})
        .unwrap();
        let client =
            catalog_replay_client(GENERATED_DATA.join("resolve/darwin_ps_incompatible.yaml")).await;

        let locked_manifest =
            Lockfile::resolve_manifest(&manifest, None, &client, &InstallableLockerMock::new())
                .await;
        if let Err(ResolveError::ResolutionFailed(res_failures)) = locked_manifest {
            // A newline is added for formatting when it's a single message
            assert_eq!(res_failures.to_string(), indoc! {"
                package 'darwin.ps' not available for
                    - aarch64-linux
                  but it is available for
                    - x86_64-darwin

                For more on managing system-specific packages, visit the documentation:
                https://flox.dev/docs/tutorials/multi-arch-environments/#handling-unsupported-packages"});
        } else {
            panic!("expected resolution failure, got {:?}", locked_manifest);
        }
    }

    #[tokio::test]
    async fn test_locking_with_store_paths() {
        let (foo_iid, foo_descriptor, foo_locked) = fake_store_path_lock("foo");
        let store_path = &foo_descriptor.store_path;
        let system = &foo_descriptor.systems.as_ref().unwrap()[0];

        let manifest: Manifest = toml::from_str(&formatdoc! {r#"
            version = 1
            [install]
            {foo_iid}.store-path = "{store_path}"
            {foo_iid}.systems = ["{system}"]
        "#})
        .unwrap();

        let client = catalog::MockClient::new();

        let resolved_packages =
            Lockfile::resolve_manifest(&manifest, None, &client, &InstallableLockerMock::new())
                .await
                .unwrap();

        assert_eq!(&resolved_packages, &[foo_locked.into()]);
    }

    /// If a manifest doesn't have `options.systems`, it defaults to locking for
    /// 4 default systems
    #[test]
    fn collect_package_groups_defaults_to_four_systems() {
        let manifest_str = indoc! {r#"
            version = 1

            [install]
            hello_install_id.pkg-path = "hello"
        "#};
        let manifest: Manifest = toml::from_str(manifest_str).unwrap();
        let package_groups: Vec<_> = Lockfile::collect_package_groups(&manifest, None)
            .unwrap()
            .collect();

        assert_eq!(package_groups.len(), 1);

        // each system is represented by a separate package descriptor
        let systems = package_groups[0]
            .descriptors
            .iter()
            .flat_map(|d| d.systems.clone())
            .collect::<Vec<_>>();

        let expected_systems = [
            PackageSystem::Aarch64Darwin,
            PackageSystem::Aarch64Linux,
            PackageSystem::X8664Darwin,
            PackageSystem::X8664Linux,
        ];

        assert_eq!(&*systems, expected_systems.as_slice());
    }

    #[test]
    fn test_split_out_fully_locked_packages() {
        let (foo_iid, foo_descriptor, foo_locked) =
            fake_catalog_package_lock("foo", Some("group1"));
        let (bar_iid, bar_descriptor, bar_locked) =
            fake_catalog_package_lock("bar", Some("group1"));
        let (baz_iid, baz_descriptor, baz_locked) =
            fake_catalog_package_lock("baz", Some("group2"));
        let (yeet_iid, yeet_descriptor, _) = fake_catalog_package_lock("yeet", Some("group2"));

        let mut manifest = Manifest::default();
        manifest
            .install
            .inner_mut()
            .insert(foo_iid, foo_descriptor.clone());
        manifest
            .install
            .inner_mut()
            .insert(bar_iid, bar_descriptor.clone());
        manifest
            .install
            .inner_mut()
            .insert(baz_iid.clone(), baz_descriptor.clone());

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: [&foo_locked, &bar_locked, &baz_locked]
                .map(|p| p.clone().into())
                .to_vec(),
            compose: None,
        };

        manifest
            .install
            .inner_mut()
            .insert(yeet_iid.clone(), yeet_descriptor.clone());

        let groups = Lockfile::collect_package_groups(&manifest, Some(&locked)).unwrap();

        let (fully_locked, to_resolve): (Vec<_>, Vec<_>) =
            Lockfile::split_fully_locked_groups(groups, Some(&locked));

        // All packages of group1 are locked
        assert_eq!(&fully_locked, &[bar_locked, foo_locked].map(Into::into));

        // Only one package of group2 is locked, so it should be in to_resolve as a group
        assert_eq!(to_resolve, vec![PackageGroup {
            name: "group2".to_string(),
            descriptors: vec![
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "baz".to_string(),
                    derivation: Some(baz_locked.derivation.clone()),
                    install_id: baz_iid,
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin,],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "yeet".to_string(),
                    derivation: None,
                    install_id: yeet_iid,
                    version: None,
                    allow_broken: None,
                    allow_insecure: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    allow_missing_builds: None,
                    systems: vec![PackageSystem::Aarch64Darwin,],
                }
            ],
        }]);
    }

    /// When packages are locked for multiple systems,
    /// locking the same package for fewer systems should drop the extra systems
    #[test]
    fn drop_packages_for_removed_systems() {
        let (foo_iid, foo_descriptor_one_system, foo_locked) =
            fake_catalog_package_lock("foo", Some("group1"));

        let systems = &foo_descriptor_one_system
            .as_catalog_descriptor_ref()
            .expect("expected a catalog descriptor")
            .systems;

        assert_eq!(
            systems,
            &Some(vec![PackageSystem::Aarch64Darwin.to_string()]),
            "`fake_package` should set the system to [`Aarch64Darwin`]"
        );

        let mut foo_descriptor_two_systems = foo_descriptor_one_system.clone();

        if let ManifestPackageDescriptor::Catalog(descriptor) = &mut foo_descriptor_two_systems {
            descriptor
                .systems
                .as_mut()
                .unwrap()
                .push(PackageSystem::Aarch64Linux.to_string());
        } else {
            panic!("Expected a catalog descriptor");
        };

        let foo_locked_second_system = LockedPackageCatalog {
            system: PackageSystem::Aarch64Linux.to_string(),
            ..foo_locked.clone()
        };

        let mut manifest = Manifest::default();
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor_two_systems.clone());

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![
                foo_locked.clone().into(),
                foo_locked_second_system.clone().into(),
            ],
            compose: None,
        };

        manifest
            .install
            .inner_mut()
            .insert(foo_iid, foo_descriptor_one_system.clone());

        let groups = Lockfile::collect_package_groups(&manifest, Some(&locked))
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].descriptors.len(), 1, "Expected only 1 descriptor");
        assert_eq!(
            groups[0].descriptors[0].systems,
            vec![PackageSystem::Aarch64Darwin,],
            "Expected only the Darwin system to be present"
        );

        let (fully_locked, to_resolve): (Vec<_>, Vec<_>) =
            Lockfile::split_fully_locked_groups(groups, Some(&locked));

        assert_eq!(fully_locked, vec![foo_locked.into()]);
        assert_eq!(to_resolve, vec![]);
    }

    /// Adding another system to a package should invalidate the entire group
    /// such that new systems are resolved with the derivation constraints
    /// of already installed systems
    #[test]
    fn invalidate_group_if_system_added() {
        let (foo_iid, foo_descriptor_one_system, foo_locked) =
            fake_catalog_package_lock("foo", Some("group1"));

        // `fake_package` sets the system to [`Aarch64Darwin`]
        let mut foo_descriptor_two_systems = foo_descriptor_one_system.clone();
        if let ManifestPackageDescriptor::Catalog(descriptor) = &mut foo_descriptor_two_systems {
            descriptor
                .systems
                .as_mut()
                .unwrap()
                .push(PackageSystem::Aarch64Linux.to_string());
        } else {
            panic!("Expected a catalog descriptor");
        };

        let mut manifest = Manifest::default();
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor_one_system.clone());

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into()],
            compose: None,
        };

        manifest
            .install
            .inner_mut()
            .insert(foo_iid, foo_descriptor_two_systems.clone());

        let groups = Lockfile::collect_package_groups(&manifest, Some(&locked))
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].descriptors.len(),
            2,
            "Expected descriptors for two systems"
        );
        assert_eq!(groups[0].descriptors[0].systems, vec![
            PackageSystem::Aarch64Darwin
        ]);
        assert_eq!(groups[0].descriptors[1].systems, vec![
            PackageSystem::Aarch64Linux
        ]);

        let (fully_locked, to_resolve): (Vec<_>, Vec<_>) =
            Lockfile::split_fully_locked_groups(groups, Some(&locked));

        assert_eq!(fully_locked, vec![]);
        assert_eq!(to_resolve.len(), 1);
    }

    /// If a flake installable is already locked, it should not be resolved again.
    /// Test that the locked package and unlocked package are correctly partitioned.
    #[test]
    fn split_out_locked_installables() {
        let system = "aarch64-darwin";
        let (foo_iid, foo_descriptor, _) = fake_flake_installable_lock("foo");
        let (bar_iid, bar_descriptor, bar_locked) = fake_flake_installable_lock("bar");

        let mut manifest = Manifest::default();
        manifest.options.systems = Some(vec![system.to_string()]);

        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone().into());
        manifest
            .install
            .inner_mut()
            .insert(bar_iid.clone(), bar_descriptor.clone().into());

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![bar_locked.clone().into()],
            compose: None,
        };

        let flake_installables = Lockfile::collect_flake_installables(&manifest);

        let (locked, to_resolve): (Vec<_>, Vec<_>) =
            Lockfile::split_locked_flake_installables(flake_installables, Some(&locked));

        assert_eq!(locked, vec![bar_locked.into()]);
        assert_eq!(&to_resolve, &[FlakeInstallableToLock {
            install_id: foo_iid.clone(),
            descriptor: foo_descriptor.clone(),
            system: system.to_string(),
        }]);
    }

    /// If the lockfile contains a package that is not in the manifest,
    /// the lock is removed.
    #[test]
    fn remove_stale_locked_installables() {
        let system = "aarch64-darwin";
        let (_, _, bar_locked) = fake_flake_installable_lock("bar");

        let mut manifest = Manifest::default();
        manifest.options.systems = Some(vec![system.to_string()]);

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![bar_locked.clone().into()],
            compose: None,
        };

        let flake_installables = Lockfile::collect_flake_installables(&manifest);

        let (locked, to_resolve): (Vec<_>, Vec<_>) =
            Lockfile::split_locked_flake_installables(flake_installables, Some(&locked));

        assert_eq!(locked, vec![]);
        assert_eq!(&to_resolve, &[]);
    }

    /// If a system is removed from the manifest,
    /// the locked package for that system should be removed.
    #[test]
    fn drop_locked_installable_for_removed_systems() {
        let system = "aarch64-darwin";
        let (foo_iid, foo_descriptor, foo_locked) = fake_flake_installable_lock("foo");

        let foo_locked_system_1 = foo_locked.clone();
        let mut foo_locked_system_2 = foo_locked;
        foo_locked_system_2.locked_installable.system = PackageSystem::Aarch64Linux.to_string();

        let mut manifest = Manifest::default();
        manifest.options.systems = Some(vec![system.to_string()]);

        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone().into());

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![
                foo_locked_system_1.clone().into(),
                foo_locked_system_2.into(),
            ],
            compose: None,
        };

        let flake_installables = Lockfile::collect_flake_installables(&manifest);

        let (locked, to_resolve): (Vec<_>, Vec<_>) =
            Lockfile::split_locked_flake_installables(flake_installables, Some(&locked));

        assert_eq!(locked, vec![foo_locked_system_1.into()]);
        assert_eq!(&to_resolve, &[]);
    }

    /// If a system is added to the manifest, the package should be reresolved for all systems
    #[test]
    fn invalidate_locked_flake_if_system_added() {
        let system_1 = "aarch64-darwin";
        let system_2 = "aarch64-linux";
        let (foo_iid, foo_descriptor, foo_locked) = fake_flake_installable_lock("foo");

        let mut manifest = Manifest::default();
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone().into());

        // lockfile for only system_1
        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.clone().into()],
            compose: None,
        };

        // system_2 is added to the manifest
        manifest.options.systems = Some(vec![system_1.to_string(), system_2.to_string()]);

        let flake_installables = Lockfile::collect_flake_installables(&manifest);

        let (locked, to_resolve): (Vec<_>, Vec<_>) =
            Lockfile::split_locked_flake_installables(flake_installables, Some(&locked));

        assert_eq!(locked, vec![]);
        assert_eq!(
            to_resolve,
            [system_1, system_2].map(|system| FlakeInstallableToLock {
                install_id: foo_iid.clone(),
                descriptor: foo_descriptor.clone(),
                system: system.to_string(),
            })
        );
    }

    /// If all packages are already locked, return without locking/resolution
    #[tokio::test]
    async fn lock_manifest_noop_if_fully_locked() {
        let (flox, _tempdir) = flox_instance();
        let (foo_iid, foo_descriptor, foo_locked) = fake_catalog_package_lock("foo", None);
        let (bar_iid, bar_descriptor, bar_locked) = fake_flake_installable_lock("bar");

        let mut manifest = Manifest::default();
        manifest.options.systems = Some(vec![PackageSystem::Aarch64Darwin.to_string()]);
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone());
        manifest
            .install
            .inner_mut()
            .insert(bar_iid.clone(), bar_descriptor.clone().into());

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into(), bar_locked.into()],
            compose: None,
        };

        let locked_manifest =
            Lockfile::lock_manifest(&flox, &manifest, Some(&locked), &IncludeFetcher {
                base_directory: None,
            })
            .await
            .unwrap();

        assert_eq!(locked_manifest, locked);
    }

    proptest! {
        // This probably isn't the best suited for proptest as there are lots of
        // writes to disk.
        // 32 cases takes .1-.2 seconds for me
        #![proptest_config(ProptestConfig::with_cases(32))]
        /// If we lock twice, the second lockfile should be the same as the first
        /// Use manifests without [install] sections so we don't have to
        /// generate resolution responses
        #[test]
        fn lock_manifest_noop_if_locked_without_install_section((flox, tempdir, environments_to_include) in generate_path_environments_without_install_or_include(3)) {
            let manifest = Manifest {
                version: 1.into(),
                include: Include {
                    environments: environments_to_include
                        .into_iter()
                        .map(|(dir, _)| IncludeDescriptor::Local {
                            dir,
                            name: None,
                        })
                        .collect(),
                },
                ..Default::default()
            };

            // Lock
            let lockfile = Lockfile::lock_manifest(&flox, &manifest, None, &IncludeFetcher {
                base_directory: Some(tempdir.path().to_path_buf()),
            })
            .block_on()
            .unwrap();

            // Lock again with a mock_include_fetcher
            let lockfile_2 =
                Lockfile::lock_manifest(&flox, &manifest, Some(&lockfile), &mock_include_fetcher())
                    .block_on()
                    .unwrap();

            prop_assert_eq!(lockfile, lockfile_2);
        }
    }

    /// If flake installables are already locked, no locking should occur.
    /// Catalog packages are still being resolved if not locked.
    #[tokio::test]
    async fn skip_flake_installables_noop_if_fully_locked() {
        let (bar_iid, bar_descriptor, bar_locked) = fake_flake_installable_lock("bar");

        let mut manifest = Manifest::default();
        manifest.options.systems = Some(vec![PackageSystem::Aarch64Darwin.to_string()]);
        manifest.install.inner_mut().insert(
            "hello".to_string(),
            ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog {
                pkg_path: "hello".to_string(),
                pkg_group: None,
                priority: None,
                version: None,
                systems: None,
                outputs: None,
            }),
        );
        manifest
            .install
            .inner_mut()
            .insert(bar_iid.clone(), bar_descriptor.clone().into());

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![bar_locked.into()],
            compose: None,
        };

        // TODO: it would probably be better to tweak
        // fake_flake_installable_lock to return a system specific descriptor
        // and use plain hello mock
        let client_mock = auto_recording_catalog_client("hello_aarch64-darwin");

        let resolved_packages =
            Lockfile::resolve_manifest(&manifest, Some(&locked), &client_mock, &PanickingLocker)
                .await
                .unwrap();

        assert_eq!(resolved_packages.len(), 2, "{:#?}", resolved_packages);
    }

    /// If catalog packages are already locked, no locking should occur.
    /// Installables are still being resolved if not locked.
    #[tokio::test]
    async fn skip_catatalog_package_if_fully_locked() {
        let (foo_iid, foo_descriptor, foo_locked) = fake_catalog_package_lock("foo", None);
        let (bar_iid, bar_descriptor, bar_locked) = fake_flake_installable_lock("bar");

        let mut manifest = Manifest::default();
        manifest.options.systems = Some(vec![PackageSystem::Aarch64Darwin.to_string()]);
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone());
        manifest
            .install
            .inner_mut()
            .insert(bar_iid.clone(), bar_descriptor.clone().into());

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into()],
            compose: None,
        };

        let locker_mock = InstallableLockerMock::new();
        locker_mock.push_lock_result(Ok(bar_locked.locked_installable));

        let resolved_packages = Lockfile::resolve_manifest(
            &manifest,
            Some(&locked),
            &MockClient::default(),
            &locker_mock,
        )
        .await
        .unwrap();

        assert_eq!(resolved_packages.len(), 2, "{:#?}", resolved_packages);
    }

    /// If catalog packages are already locked, no locking should occur.
    /// Installables are still being resolved if not locked.
    #[tokio::test]
    async fn update_priority_if_fully_locked() {
        let (foo_iid, foo_descriptor, foo_locked) = fake_catalog_package_lock("foo", None);

        let mut manifest = Manifest::default();
        manifest.options.systems = Some(vec![PackageSystem::Aarch64Darwin.to_string()]);
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone());

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.clone().into()],
            compose: None,
        };

        let mut foo_descriptor_priority_after = foo_descriptor.unwrap_catalog_descriptor().unwrap();
        foo_descriptor_priority_after.priority = Some(1);

        let mut foo_locked_priority_after = foo_locked.clone();
        foo_locked_priority_after.priority = 1;

        let mut manifest_pririty_after = manifest.clone();
        manifest_pririty_after.install.inner_mut().insert(
            foo_iid.clone(),
            foo_descriptor_priority_after.clone().into(),
        );

        let locker_mock = InstallableLockerMock::new();
        let resolved_packages = Lockfile::resolve_manifest(
            &manifest_pririty_after,
            Some(&locked),
            &MockClient::default(),
            &locker_mock,
        )
        .await
        .unwrap();

        assert_eq!(resolved_packages.as_slice(), &[
            foo_locked_priority_after.into()
        ]);
    }

    /// [Lockfile::lock_manifest] returns an error if an already
    /// locked package is no longer allowed
    #[tokio::test]
    async fn lock_manifest_catches_not_allowed_package() {
        // Create a manifest and lockfile with an unfree package foo.
        // Don't set `options.allow.unfree`
        let (foo_iid, foo_descriptor_one_system, mut foo_locked) =
            fake_catalog_package_lock("foo", None);
        foo_locked.unfree = Some(true);
        let mut manifest = Manifest::default();
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor_one_system.clone());

        let locked = Lockfile {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into()],
            compose: None,
        };

        // Set `options.allow.unfree = false` in the manifest, but not the lockfile
        manifest.options.allow.unfree = Some(false);

        let client = catalog::MockClient::new();
        assert!(matches!(
            Lockfile::resolve_manifest(
                &manifest,
                Some(&locked),
                &client,
                &InstallableLockerMock::new()
            )
            .await
            .unwrap_err(),
            ResolveError::UnfreeNotAllowed { .. }
        ));
    }

    /// [Lockfile::lock_manifest] returns an error if the server
    /// returns a package that is not allowed.
    #[tokio::test]
    async fn lock_manifest_catches_not_allowed_package_from_server() {
        let manifest = Manifest::from_str(indoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello"

            [options]
            allow.unfree = false
        "#})
        .unwrap();

        // Return a response that says foo is unfree.
        // If this happens, it's a bug in the server, which is why we don't use
        // a mock for this test.
        let client = catalog_replay_client(
            GENERATED_DATA.join("resolve/hello_buggy_unfree_server_response.yaml"),
        )
        .await;
        assert!(matches!(
            Lockfile::resolve_manifest(&manifest, None, &client, &InstallableLockerMock::new())
                .await
                .unwrap_err(),
            ResolveError::UnfreeNotAllowed { .. }
        ));
    }

    /// [Lockfile::check_packages_are_allowed] returns an error
    /// when it finds a disallowed license
    #[test]
    fn check_packages_are_allowed_disallowed_license() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.license = Some("disallowed".to_string());

        assert!(matches!(
            Lockfile::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: None,
                licenses: Some(vec!["allowed".to_string()])
            }),
            Err(ResolveError::LicenseNotAllowed { .. })
        ));
    }

    /// [Lockfile::check_packages_are_allowed] does not error when
    /// a package's license is allowed
    #[test]
    fn check_packages_are_allowed_allowed_license() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.license = Some("allowed".to_string());

        assert!(
            Lockfile::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: None,
                licenses: Some(vec!["allowed".to_string()])
            })
            .is_ok()
        );
    }

    /// [Lockfile::check_packages_are_allowed] allows any license if allowed
    /// licenses is empty
    #[test]
    fn check_packages_are_allowed_for_empty_licenses() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.license = Some("allowed".to_string());

        assert!(
            Lockfile::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: None,
                licenses: Some(vec![]),
            })
            .is_ok()
        );
    }

    /// [Lockfile::check_packages_are_allowed] returns an error
    /// when a package is broken even if `allow.broken` is unset
    #[test]
    fn check_packages_are_allowed_broken_default() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.broken = Some(true);

        assert!(matches!(
            Lockfile::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: None,
                licenses: None
            }),
            Err(ResolveError::BrokenNotAllowed { .. })
        ));
    }

    /// [Lockfile::check_packages_are_allowed] does not error for a
    /// broken package when `allow.broken = true`
    #[test]
    fn check_packages_are_allowed_broken_true() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.broken = Some(true);

        assert!(
            Lockfile::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: Some(true),
                licenses: None
            })
            .is_ok()
        );
    }

    /// [Lockfile::check_packages_are_allowed] returns an error
    /// when a package is broken and `allow.broken = false`
    #[test]
    fn check_packages_are_allowed_broken_false() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.broken = Some(true);

        assert!(matches!(
            Lockfile::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: Some(false),
                licenses: None
            }),
            Err(ResolveError::BrokenNotAllowed { .. })
        ));
    }

    /// [Lockfile::check_packages_are_allowed] does not error for
    /// an unfree package when `allow.unfree` is unset
    #[test]
    fn check_packages_are_allowed_unfree_default() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.unfree = Some(true);

        assert!(
            Lockfile::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: None,
                licenses: None
            })
            .is_ok()
        );
    }

    /// [Lockfile::check_packages_are_allowed] does not error for a
    /// an unfree package when `allow.unfree = true`
    #[test]
    fn check_packages_are_allowed_unfree_true() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.unfree = Some(true);

        assert!(
            Lockfile::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: Some(true),
                broken: None,
                licenses: None
            })
            .is_ok()
        );
    }

    /// [Lockfile::check_packages_are_allowed] returns an error
    /// when a package is unfree and `allow.unfree = false`
    #[test]
    fn check_packages_are_allowed_unfree_false() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.unfree = Some(true);

        assert!(matches!(
            Lockfile::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: Some(false),
                broken: None,
                licenses: None
            }),
            Err(ResolveError::UnfreeNotAllowed { .. })
        ));
    }

    #[test]
    fn test_list_packages_catalog() {
        let (foo_iid, foo_descriptor, foo_locked) =
            fake_catalog_package_lock("foo", Some("group1"));
        let (bar_iid, bar_descriptor, bar_locked) =
            fake_catalog_package_lock("bar", Some("group1"));
        let (baz_iid, mut baz_descriptor, mut baz_locked) =
            fake_catalog_package_lock("baz", Some("group2"));

        if let ManifestPackageDescriptor::Catalog(ref mut descriptor) = baz_descriptor {
            descriptor.systems = Some(vec![PackageSystem::Aarch64Linux.to_string()]);
        } else {
            panic!("Expected a catalog descriptor");
        };
        baz_locked.system = PackageSystem::Aarch64Linux.to_string();

        let mut manifest = Manifest::default();
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone());
        manifest
            .install
            .inner_mut()
            .insert(bar_iid.clone(), bar_descriptor.clone());
        manifest
            .install
            .inner_mut()
            .insert(baz_iid.clone(), baz_descriptor.clone());

        let locked = Lockfile {
            version: Version::<1>,
            manifest,
            packages: vec![
                foo_locked.clone().into(),
                bar_locked.clone().into(),
                baz_locked.clone().into(),
            ],
            compose: None,
        };

        let actual = locked
            .list_packages(&PackageSystem::Aarch64Darwin.to_string())
            .unwrap();
        let expected = [
            PackageToList::Catalog(
                foo_descriptor.unwrap_catalog_descriptor().unwrap(),
                foo_locked,
            ),
            PackageToList::Catalog(
                bar_descriptor.unwrap_catalog_descriptor().unwrap(),
                bar_locked,
            ),
            // baz is not in the list because it is not available for the requested system
        ];

        assert_eq!(&actual, &expected);
    }

    #[test]
    fn test_list_packages_flake() {
        let (foo_iid, foo_descriptor, foo_locked) = fake_flake_installable_lock("foo");
        let (baz_iid, baz_descriptor, mut baz_locked) = fake_flake_installable_lock("baz");

        baz_locked.locked_installable.system = PackageSystem::Aarch64Linux.to_string();

        let mut manifest = Manifest::default();
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone().into());
        manifest
            .install
            .inner_mut()
            .insert(baz_iid.clone(), baz_descriptor.into());

        let locked = Lockfile {
            version: Version::<1>,
            manifest,
            packages: vec![foo_locked.clone().into(), baz_locked.clone().into()],
            compose: None,
        };

        let actual = locked
            .list_packages(&PackageSystem::Aarch64Darwin.to_string())
            .unwrap();
        let expected = [
            PackageToList::Flake(foo_descriptor, foo_locked), // baz is not in the list because it is not available for the requested system
        ];

        assert_eq!(&actual, &expected);
    }

    #[test]
    fn test_list_packages_store_path() {
        let (foo_iid, foo_descriptor, foo_locked) = fake_store_path_lock("foo");
        let (baz_iid, baz_descriptor, mut baz_locked) = fake_store_path_lock("baz");

        baz_locked.system = PackageSystem::Aarch64Linux.to_string();

        let mut manifest = Manifest::default();
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone().into());
        manifest
            .install
            .inner_mut()
            .insert(baz_iid.clone(), baz_descriptor.into());

        let locked = Lockfile {
            version: Version::<1>,
            manifest,
            packages: vec![foo_locked.clone().into(), baz_locked.clone().into()],
            compose: None,
        };

        let actual = locked
            .list_packages(&PackageSystem::Aarch64Darwin.to_string())
            .unwrap();
        let expected = [
            PackageToList::StorePath(foo_locked), // baz is not in the list because it is not available for the requested system
        ];

        assert_eq!(&actual, &expected);
    }

    #[test]
    fn respects_flake_descriptor_systems() {
        let manifest_contents = formatdoc! {r#"
        version = 1

        [install]
        bpftrace.flake = "github:NixOS/nixpkgs#bpftrace"
        bpftrace.systems = ["x86_64-linux"]

        [options]
        systems = ["aarch64-linux", "x86_64-linux"]
        "#};
        let manifest = toml_edit::de::from_str(&manifest_contents).unwrap();
        let installables = Lockfile::collect_flake_installables(&manifest).collect::<Vec<_>>();
        assert_eq!(installables.len(), 1);
        assert_eq!(installables[0].system.as_str(), "x86_64-linux");
    }

    /// [Lockfile::merge_manifest] fetches an included environment when it is
    /// not already locked
    #[test]
    fn merge_manifest_fetches_included_environment() {
        let (flox, tempdir) = flox_instance();
        let manifest_contents = indoc! {r#"
        version = 1

        [include]
        environments = [
          { dir = "dep1" }
        ]
        "#};
        let manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        // Create dep1 environment
        let dep1_path = tempdir.path().join("dep1");
        let dep1_manifest_contents = indoc! {r#"
        version = 1

        [vars]
        foo = "dep1"
        "#};

        fs::create_dir(&dep1_path).unwrap();
        let mut dep1 = new_path_environment_in(&flox, dep1_manifest_contents, &dep1_path);
        dep1.lockfile(&flox).unwrap();

        // Merge
        let (merged, compose) = Lockfile::merge_manifest(
            &flox,
            &manifest,
            None,
            &IncludeFetcher {
                base_directory: Some(tempdir.path().to_path_buf()),
            },
            ManifestMerger::Shallow(ShallowMerger),
            None,
        )
        .unwrap();

        assert_eq!(merged, Manifest {
            version: 1.into(),
            vars: Vars(BTreeMap::from([("foo".to_string(), "dep1".to_string())])),
            ..Default::default()
        });
        assert_eq!(
            compose.unwrap().include[0].manifest,
            toml_edit::de::from_str(dep1_manifest_contents).unwrap()
        )
    }

    /// [Lockfile::merge_manifest] preserves precedence of includes
    #[test]
    fn merge_manifest_preserves_include_order() {
        let (flox, tempdir) = flox_instance();
        let manifest_contents = indoc! {r#"
        version = 1

        [include]
        environments = [
          { dir = "lowest_precedence" },
          { dir = "higher_precedence" }
        ]

        [vars]
        foo = "highest_precedence"
        "#};
        let manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        // Create lowest_precedence environment
        let lowest_precedence_path = tempdir.path().join("lowest_precedence");
        let lowest_precedence_manifest_contents = indoc! {r#"
        version = 1

        [vars]
        foo = "lowest_precedence"
        bar = "lowest_precedence"
        "#};
        fs::create_dir(&lowest_precedence_path).unwrap();
        let mut lowest_precedence = new_path_environment_in(
            &flox,
            lowest_precedence_manifest_contents,
            &lowest_precedence_path,
        );
        lowest_precedence.lockfile(&flox).unwrap();

        // Create higher precedence environment
        let higher_precedence_path = tempdir.path().join("higher_precedence");
        let higher_precedence_manifest_contents = indoc! {r#"
        version = 1

        [vars]
        foo = "higher_precedence"
        bar = "higher_precedence"
        "#};
        fs::create_dir(&higher_precedence_path).unwrap();
        let mut higher_precedence = new_path_environment_in(
            &flox,
            higher_precedence_manifest_contents,
            &higher_precedence_path,
        );
        higher_precedence.lockfile(&flox).unwrap();

        // Merge
        let (merged, compose) = Lockfile::merge_manifest(
            &flox,
            &manifest,
            None,
            &IncludeFetcher {
                base_directory: Some(tempdir.path().to_path_buf()),
            },
            ManifestMerger::Shallow(ShallowMerger),
            None,
        )
        .unwrap();

        assert_eq!(merged, Manifest {
            version: 1.into(),
            vars: Vars(BTreeMap::from([
                ("foo".to_string(), "highest_precedence".to_string()),
                ("bar".to_string(), "higher_precedence".to_string())
            ])),
            ..Default::default()
        });
        assert_eq!(
            compose.as_ref().unwrap().include[0].manifest,
            toml_edit::de::from_str(lowest_precedence_manifest_contents).unwrap()
        );
        assert_eq!(
            compose.unwrap().include[1].manifest,
            toml_edit::de::from_str(higher_precedence_manifest_contents).unwrap()
        );
    }

    /// Skipping fetching an already fetched environment shouldn't break
    /// precedence.
    ///
    /// Suppose an environment starts out with a single included environment,
    /// middle_precedence,
    /// and the environment is merged.
    /// Then suppose two other environments lowest_precedence and
    /// highest_precedence are added.
    /// Precedence should still reflect the order of included environments.
    #[tokio::test]
    async fn merge_manifest_respects_precedence_when_skipping_fetch() {
        let (flox, tempdir) = flox_instance();

        let manifest_contents = indoc! {r#"
        version = 1

        [include]
        environments = [
          { dir = "middle_precedence" }
        ]
        "#};
        let manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        // Create middle_precedence environment
        let middle_precedence_path = tempdir.path().join("middle_precedence");
        let middle_precedence_manifest_contents = indoc! {r#"
        version = 1

        [vars]
        foo = "middle_precedence"
        "#};
        fs::create_dir(&middle_precedence_path).unwrap();
        let mut middle_precedence = new_path_environment_in(
            &flox,
            middle_precedence_manifest_contents,
            &middle_precedence_path,
        );
        middle_precedence.lockfile(&flox).unwrap();

        // Lock
        let include_fetcher = IncludeFetcher {
            base_directory: Some(tempdir.path().to_path_buf()),
        };

        let lockfile = Lockfile::lock_manifest(&flox, &manifest, None, &include_fetcher)
            .await
            .unwrap();

        // Edit manifest to include two more includes
        let manifest_contents = indoc! {r#"
        version = 1

        [include]
        environments = [
          { dir = "lowest_precedence" },
          { dir = "middle_precedence" },
          { dir = "highest_precedence" },
        ]
        "#};
        let manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        // Create lowest_precedence environment
        let lowest_precedence_path = tempdir.path().join("lowest_precedence");
        let lowest_precedence_manifest_contents = indoc! {r#"
        version = 1

        [vars]
        foo = "lowest_precedence"
        "#};
        fs::create_dir(&lowest_precedence_path).unwrap();
        let mut lowest_precedence = new_path_environment_in(
            &flox,
            lowest_precedence_manifest_contents,
            &lowest_precedence_path,
        );
        lowest_precedence.lockfile(&flox).unwrap();

        // Create highest_precedence environment
        let highest_precedence_path = tempdir.path().join("highest_precedence");
        let highest_precedence_manifest_contents = indoc! {r#"
        version = 1

        [vars]
        foo = "highest_precedence"
        "#};
        fs::create_dir(&highest_precedence_path).unwrap();
        let mut highest_precedence = new_path_environment_in(
            &flox,
            highest_precedence_manifest_contents,
            &highest_precedence_path,
        );
        highest_precedence.lockfile(&flox).unwrap();

        // Merge
        let (merged, compose) = Lockfile::merge_manifest(
            &flox,
            &manifest,
            Some(&lockfile),
            &include_fetcher,
            ManifestMerger::Shallow(ShallowMerger),
            None,
        )
        .unwrap();

        assert_eq!(merged, Manifest {
            version: 1.into(),
            vars: Vars(BTreeMap::from([(
                "foo".to_string(),
                "highest_precedence".to_string()
            ),])),
            ..Default::default()
        });
        assert_eq!(
            compose.as_ref().unwrap().include[0].manifest,
            toml_edit::de::from_str(lowest_precedence_manifest_contents).unwrap()
        );
        assert_eq!(
            compose.as_ref().unwrap().include[1].manifest,
            toml_edit::de::from_str(middle_precedence_manifest_contents).unwrap()
        );
        assert_eq!(
            compose.as_ref().unwrap().include[2].manifest,
            toml_edit::de::from_str(highest_precedence_manifest_contents).unwrap()
        );
    }

    /// Re-merge after editing an included environment
    /// If modify_include_descriptor is true, modify the include descriptor
    /// which should trigger a re-fetch.
    /// Otherwise, re-merging should not re-fetch.
    async fn re_merge_after_editing_dep(modify_include_descriptor: bool) {
        let (flox, tempdir) = flox_instance();

        let mut manifest_contents = indoc! {r#"
        version = 1

        [include]
        environments = [
          { dir = "dep1" }
        ]
        "#};
        let mut manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        // Create dep1 environment
        let dep1_path = tempdir.path().join("dep1");
        let dep1_manifest_contents = indoc! {r#"
        version = 1

        [vars]
        foo = "dep1"
        "#};
        let dep1_manifest = toml_edit::de::from_str(dep1_manifest_contents).unwrap();

        fs::create_dir(&dep1_path).unwrap();
        let mut dep1 = new_path_environment_in(&flox, dep1_manifest_contents, &dep1_path);
        dep1.lockfile(&flox).unwrap();

        // Lock
        let include_fetcher = IncludeFetcher {
            base_directory: Some(tempdir.path().to_path_buf()),
        };

        let lockfile = Lockfile::lock_manifest(&flox, &manifest, None, &include_fetcher)
            .await
            .unwrap();

        assert_eq!(lockfile.manifest, Manifest {
            version: 1.into(),
            vars: Vars(BTreeMap::from([("foo".to_string(), "dep1".to_string())])),
            ..Default::default()
        });
        assert_eq!(
            lockfile.compose.as_ref().unwrap().include[0].manifest,
            toml_edit::de::from_str(dep1_manifest_contents).unwrap()
        );

        // Edit dep1 and then change its name in the include descriptor and re-merge
        let dep1_edited_manifest_contents = indoc! {r#"
        version = 1

        [vars]
        foo = "dep1 edited"
        "#};
        let dep1_edited_manifest = toml_edit::de::from_str(dep1_edited_manifest_contents).unwrap();

        dep1.edit(&flox, dep1_edited_manifest_contents.to_string())
            .unwrap();

        if modify_include_descriptor {
            manifest_contents = indoc! {r#"
            version = 1

            [include]
            environments = [
              { dir = "dep1", name = "dep1 edited" }
            ]
            "#};
            manifest = toml_edit::de::from_str(manifest_contents).unwrap();
        }

        // Merge
        let (merged, compose) = Lockfile::merge_manifest(
            &flox,
            &manifest,
            Some(&lockfile),
            &include_fetcher,
            ManifestMerger::Shallow(ShallowMerger),
            None,
        )
        .unwrap();

        assert_eq!(merged, Manifest {
            version: 1.into(),
            vars: Vars(BTreeMap::from([(
                "foo".to_string(),
                if modify_include_descriptor {
                    "dep1 edited".to_string()
                } else {
                    "dep1".to_string()
                }
            )])),
            ..Default::default()
        });
        assert_eq!(
            compose.unwrap().include[0].manifest,
            if modify_include_descriptor {
                dep1_edited_manifest
            } else {
                dep1_manifest
            }
        );
    }

    /// If included environments have already been locked, the existing locked include should be used
    #[tokio::test]
    async fn merge_manifest_does_not_refetch_if_include_descriptor_unchanged() {
        re_merge_after_editing_dep(false).await;
    }

    /// [Lockfile::merge_manifest] re-fetches if any part of an include
    /// descriptor has changed
    #[tokio::test]
    async fn merge_manifest_refetches_if_include_descriptor_changed() {
        re_merge_after_editing_dep(true).await;
    }

    /// [Lockfile::merge_manifest] doesn't leave stale locked includes
    #[tokio::test]
    async fn merge_manifest_removes_stale_locked_includes() {
        let (flox, tempdir) = flox_instance();

        let mut manifest_contents = indoc! {r#"
        version = 1

        [include]
        environments = [
          { dir = "dep1" }
        ]
        "#};
        let mut manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        // Create dep1 environment
        let dep1_path = tempdir.path().join("dep1");
        let dep1_manifest_contents = indoc! {r#"
        version = 1

        [vars]
        foo = "dep1"
        "#};
        let dep1_manifest = toml_edit::de::from_str(dep1_manifest_contents).unwrap();

        fs::create_dir(&dep1_path).unwrap();
        let mut dep1 = new_path_environment_in(&flox, dep1_manifest_contents, &dep1_path);
        dep1.lockfile(&flox).unwrap();

        // Lock
        let include_fetcher = IncludeFetcher {
            base_directory: Some(tempdir.path().to_path_buf()),
        };

        let lockfile = Lockfile::lock_manifest(&flox, &manifest, None, &include_fetcher)
            .await
            .unwrap();

        assert_eq!(lockfile.manifest, Manifest {
            version: 1.into(),
            vars: Vars(BTreeMap::from([("foo".to_string(), "dep1".to_string())])),
            ..Default::default()
        });
        assert_eq!(
            lockfile.compose.as_ref().unwrap().include[0].manifest,
            dep1_manifest,
        );

        // Remove the include of dep1
        manifest_contents = indoc! {r#"
        version = 1
        "#};
        manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        // Merge
        let (merged, compose) = Lockfile::merge_manifest(
            &flox,
            &manifest,
            Some(&lockfile),
            &include_fetcher,
            ManifestMerger::Shallow(ShallowMerger),
            None,
        )
        .unwrap();

        assert_eq!(merged, manifest);

        assert!(compose.is_none());
    }

    /// [Lockfile::merge_manifest] errors if locked include names are not unique
    #[test]
    fn merge_manifest_errors_for_non_unique_include_names() {
        let (flox, tempdir) = flox_instance();

        let manifest_contents = indoc! {r#"
        version = 1

        [include]
        environments = [
          { dir = "dep1" },
          { dir = "dep2" }
        ]
        "#};
        let manifest = toml_edit::de::from_str(manifest_contents).unwrap();

        // Create dep1 named dep
        let dep1_path = tempdir.path().join("dep1");
        let dep1_manifest_contents = indoc! {r#"
        version = 1
        "#};
        fs::create_dir(&dep1_path).unwrap();
        let mut dep1 =
            new_named_path_environment_in(&flox, dep1_manifest_contents, &dep1_path, "dep");
        dep1.lockfile(&flox).unwrap();

        // Create dep2 named dep
        let dep2_path = tempdir.path().join("dep2");
        let dep2_manifest_contents = indoc! {r#"
        version = 1
        "#};
        fs::create_dir(&dep2_path).unwrap();
        let mut dep2 =
            new_named_path_environment_in(&flox, dep2_manifest_contents, &dep2_path, "dep");
        dep2.lockfile(&flox).unwrap();

        // Merge
        let include_fetcher = IncludeFetcher {
            base_directory: Some(tempdir.path().to_path_buf()),
        };

        let err = Lockfile::merge_manifest(
            &flox,
            &manifest,
            None,
            &include_fetcher,
            ManifestMerger::Shallow(ShallowMerger),
            None,
        )
        .unwrap_err();

        let RecoverableMergeError::Catchall(message) = err else {
            panic!();
        };
        assert_eq!(
            message,
            indoc! {"multiple environments in include.environments have the name 'dep'
            A unique name can be provided with the 'name' field."}
        );
    }

    #[test]
    fn merge_manifest_uses_the_merged_manifests_of_includes() {
        let env_ref = RemoteEnvironmentRef::new("owner", "name").unwrap();
        let (flox, _tempdir) = flox_instance_with_optional_floxhub(Some(env_ref.owner()));

        // Create two "child" environments, local and remote.
        let mut dep_child_local = new_path_environment(&flox, indoc! {r#"
                version = 1

                [vars]
                child_local = "hi"
            "#});
        dep_child_local.lockfile(&flox).unwrap();

        let _dep_child_remote = mock_remote_environment(
            &flox,
            indoc! {r#"
                version = 1

                [vars]
                child_remote = "hi"
            "#},
            env_ref.owner().clone(),
            Some(&env_ref.name().to_string()),
        );

        // Create a "parent" environment that includes the local and remote "children".
        let mut dep_parent = new_path_environment(&flox, &formatdoc! {r#"
                version = 1

                [vars]
                parent = "hi"

                [include]
                environments = [
                    {{ dir = "{include_path}" }},
                    {{ remote = "owner/name" }},
                ]
            "#, include_path = dep_child_local.parent_path().unwrap().to_string_lossy() });
        dep_parent.lockfile(&flox).unwrap();

        // Create a composer environment that indirectly includes the "children".
        let mut composer = new_path_environment(&flox, &formatdoc! {r#"
                version = 1

                [vars]
                composer = "hi"

                [include]
                environments = [
                    {{ dir = "{include_path}" }},
                ]
            "#, include_path = &dep_parent.parent_path().unwrap().to_string_lossy() });

        let lockfile: Lockfile = composer.lockfile(&flox).unwrap().into();
        assert_eq!(
            lockfile.manifest,
            Manifest {
                vars: Vars(BTreeMap::from([
                    ("child_local".to_string(), "hi".to_string()),
                    ("child_remote".to_string(), "hi".to_string()),
                    ("parent".to_string(), "hi".to_string()),
                    ("composer".to_string(), "hi".to_string()),
                ])),
                ..Default::default()
            },
            "composer should include fields from both indirect child includes"
        )
    }
}
