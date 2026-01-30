use std::sync::LazyLock;

mod catalog;
mod compose;
mod flake;
mod store_path;

use flox_core::data::{CanonicalPath, System};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type FlakeRef = Value;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::fs;
use std::str::FromStr;

use flox_core::Version;

use crate::lockfile::catalog::LockedPackageCatalog;
use crate::lockfile::compose::Compose;
use crate::lockfile::flake::LockedPackageFlake;
use crate::lockfile::store_path::LockedPackageStorePath;
use crate::parsed::latest::{
    ManifestPackageDescriptor,
    PackageDescriptorCatalog,
    PackageDescriptorFlake,
};
use crate::parsed::{Inner, PackageLookup};
use crate::{Deserialized, Manifest, ManifestError, MigratedManifest};

#[derive(Debug, thiserror::Error)]
pub enum LockfileError {
    #[error("failed to parse lockfile JSON: {0}")]
    ParseJson(#[source] serde_json::Error),

    #[error("failed to read lockfile: {0}")]
    IORead(#[source] std::io::Error),

    #[error("failed to write lockfile: {0}")]
    IOWrite(#[source] std::io::Error),

    #[error("corrupt manifest; couldn't find package descriptor for locked install_id '{0}'")]
    MissingPackageDescriptor(String),

    #[error(transparent)]
    Manifest(#[from] ManifestError),
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
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct Lockfile {
    #[serde(rename = "lockfile-version")]
    pub version: Version<1>,
    /// The manifest that was locked.
    ///
    /// For an environment that doesn't include any others, this is the `manifest.toml`
    /// on disk at lock-time. For an environment that *does* include others, this is
    /// the merged manifest that was locked.
    pub manifest: Manifest<Deserialized>,
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

    /// Convert a locked manifest to a list of installed packages for a given system.
    pub fn list_packages(&self, system: &System) -> Result<Vec<PackageToList>, LockfileError> {
        let manifest = self
            .manifest
            .migrate_deserialized(&self)
            .map_err(LockfileError::Manifest)?;
        let manifest = manifest.migrated_manifest();
        self.packages
            .iter()
            .filter(|package| package.system() == system)
            .cloned()
            .map(|package| match package {
                LockedPackage::Catalog(pkg) => {
                    let descriptor = manifest.pkg_descriptor_with_id(&pkg.install_id).ok_or(
                        LockfileError::MissingPackageDescriptor(pkg.install_id.clone()),
                    )?;

                    let Some(descriptor) = descriptor.unwrap_catalog_descriptor() else {
                        Err(LockfileError::MissingPackageDescriptor(
                            pkg.install_id.clone(),
                        ))?
                    };

                    Ok(PackageToList::Catalog(descriptor, pkg))
                },
                LockedPackage::Flake(locked_package) => {
                    let descriptor = manifest
                        .pkg_descriptor_with_id(&locked_package.install_id)
                        .ok_or(LockfileError::MissingPackageDescriptor(
                            locked_package.install_id.clone(),
                        ))?;

                    let Some(descriptor) = descriptor.unwrap_flake_descriptor() else {
                        Err(LockfileError::MissingPackageDescriptor(
                            locked_package.install_id.clone(),
                        ))?
                    };

                    Ok(PackageToList::Flake(descriptor, locked_package))
                },
                LockedPackage::StorePath(locked) => Ok(PackageToList::StorePath(locked)),
            })
            .collect::<Result<Vec<_>, LockfileError>>()
    }

    /// The manifest the user edits (i.e. not merged)
    pub fn user_manifest(&self) -> &Manifest<Deserialized> {
        match &self.compose {
            Some(compose) => &compose.composer,
            None => &self.manifest,
        }
    }

    /// Transform a lockfile into a mapping that is easier to query:
    /// Lockfile -> { (install_id, system): (package_descriptor, locked_package) }
    fn make_seed_mapping(
        seed: &Lockfile,
    ) -> Result<HashMap<(&str, &str), (ManifestPackageDescriptor, LockedPackage)>, LockfileError>
    {
        let manifest = seed
            .manifest
            .migrate_deserialized(&seed)
            .map_err(LockfileError::Manifest)?;
        let manifest = manifest.migrated_manifest();
        Ok(seed
            .packages
            .iter()
            .filter_map(|locked| {
                let system = locked.system().as_str();
                let install_id = locked.install_id();
                let descriptor = manifest.install.inner().get(locked.install_id())?;
                Some(((install_id, system), (descriptor.clone(), locked.clone())))
            })
            .collect())
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
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
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
    use catalog_api_v1::types as catalog_types;

    use super::*;
    use crate::lockfile::flake::LockedInstallable;
    use crate::parsed::common::{DEFAULT_GROUP_NAME, DEFAULT_PRIORITY, PackageDescriptorStorePath};
    use crate::parsed::latest::{ManifestPackageDescriptor, PackageDescriptorCatalog};

    pub fn fake_catalog_package_lock(
        name: &str,
        group: Option<&str>,
    ) -> (String, ManifestPackageDescriptor, LockedPackageCatalog) {
        let install_id = format!("{}_install_id", name);

        let descriptor = PackageDescriptorCatalog {
            pkg_path: name.to_string(),
            pkg_group: group.map(|s| s.to_string()),
            systems: Some(vec![
                catalog_types::PackageSystem::Aarch64Darwin.to_string(),
            ]),
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
            system: catalog_types::PackageSystem::Aarch64Darwin.to_string(),
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
            systems: Some(vec![
                catalog_types::PackageSystem::Aarch64Darwin.to_string(),
            ]),
            priority: None,
        };

        let locked = LockedPackageStorePath {
            install_id: install_id.clone(),
            store_path: format!("/nix/store/{}", name),
            system: catalog_types::PackageSystem::Aarch64Darwin.to_string(),
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

    use catalog_api_v1::types::PackageSystem;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use test_helpers::{
        fake_catalog_package_lock,
        fake_flake_installable_lock,
        fake_store_path_lock,
    };
    use toml_edit::DocumentMut;

    use super::*;
    use crate::parsed::latest::ManifestLatest;

    static TEST_MANIFEST_CONTENTS: &str = indoc! {r#"
          version = 1

          [install]
          hello_install_id.pkg-path = "hello"
          hello_install_id.pkg-group = "group"

          [options]
          systems = ["aarch64-darwin"]
        "#};
    static TEST_RAW_MANIFEST: LazyLock<DocumentMut> = LazyLock::new(|| {
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

    static TEST_TYPED_MANIFEST: LazyLock<Manifest<Deserialized>> =
        LazyLock::new(|| toml_edit::de::from_str(TEST_MANIFEST_CONTENTS).unwrap());

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

        let mut manifest = ManifestLatest::default();
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
        let manifest = Manifest::<Deserialized>::from_latest(manifest);

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

        let mut manifest = ManifestLatest::default();
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone().into());
        manifest
            .install
            .inner_mut()
            .insert(baz_iid.clone(), baz_descriptor.into());
        let manifest = Manifest::<Deserialized>::from_latest(manifest);

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

        let mut manifest = ManifestLatest::default();
        manifest
            .install
            .inner_mut()
            .insert(foo_iid.clone(), foo_descriptor.clone().into());
        manifest
            .install
            .inner_mut()
            .insert(baz_iid.clone(), baz_descriptor.into());
        let manifest = Manifest::<Deserialized>::from_latest(manifest);

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
}
