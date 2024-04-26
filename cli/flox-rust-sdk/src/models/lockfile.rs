use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type FlakeRef = Value;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use log::debug;
use thiserror::Error;

use super::container_builder::ContainerBuilder;
use super::environment::UpdateResult;
use super::manifest::{ManifestPackageDescriptor, TypedManifestCatalog, DEFAULT_GROUP_NAME};
use super::pkgdb::CallPkgDbError;
use crate::data::{CanonicalPath, CanonicalizeError, System, Version};
use crate::flox::Flox;
use crate::models::environment::{global_manifest_lockfile_path, global_manifest_path};
use crate::models::pkgdb::{call_pkgdb, BuildEnvResult, PKGDB_BIN};
use crate::providers::catalog::{
    self,
    CatalogPage,
    PackageDescriptor,
    PackageGroup,
    ResolvedPackageGroup,
};
use crate::utils::CommandExt;

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

#[derive(Debug, Clone, PartialEq, Serialize /* , Deserialize implemented manually */)]
#[serde(untagged)]
pub enum LockedManifest {
    Catalog(LockedManifestCatalog),
    Pkgdb(LockedManifestPkgdb),
}

impl<'de> Deserialize<'de> for LockedManifest {
    fn deserialize<D>(deserializer: D) -> Result<LockedManifest, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let version = value.get("lockfile-version").and_then(Value::as_u64);

        match version {
            Some(0) => Ok(LockedManifest::Pkgdb(LockedManifestPkgdb(value))),
            Some(1) => serde_json::from_value(value)
                .map(LockedManifest::Catalog)
                .map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom(
                "unsupported or missing 'lockfile-version'",
            )),
        }
    }
}

impl LockedManifest {
    /// Build a locked manifest
    ///
    /// If a gcroot_out_link_path is provided,
    /// the environment will be linked to that path and a gcroot will be created
    pub fn build(
        &self,
        pkgdb: &Path,
        gcroot_out_link_path: Option<&Path>,
        store_path: &Option<PathBuf>,
    ) -> Result<PathBuf, LockedManifestError> {
        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd.arg("buildenv").arg(&self.to_string());

        if let Some(gcroot_out_link_path) = gcroot_out_link_path {
            pkgdb_cmd.args(["--out-link", &gcroot_out_link_path.to_string_lossy()]);
            if let Some(store_path) = store_path {
                pkgdb_cmd.args(["--store-path", &store_path.to_string_lossy()]);
            }
        }

        debug!("building environment with command: {}", pkgdb_cmd.display());

        let result: BuildEnvResult =
            serde_json::from_value(call_pkgdb(pkgdb_cmd).map_err(LockedManifestError::BuildEnv)?)
                .map_err(LockedManifestError::ParseBuildEnvOutput)?;

        Ok(PathBuf::from(result.store_path))
    }

    /// Build a container image from a locked manifest
    /// and write it to a provided sink.
    ///
    /// The sink can be e.g. a [File](std::fs::File), [Stdout](std::io::Stdout),
    /// or an internal buffer.
    pub fn build_container(&self, pkgdb: &Path) -> Result<ContainerBuilder, LockedManifestError> {
        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd
            .arg("buildenv")
            .arg("--container")
            .arg(&self.to_string());

        debug!(
            "building container builder with command: {}",
            pkgdb_cmd.display()
        );
        let result: BuildEnvResult =
            serde_json::from_value(call_pkgdb(pkgdb_cmd).map_err(LockedManifestError::BuildEnv)?)
                .map_err(LockedManifestError::ParseBuildEnvOutput)?;

        let container_builder_path = PathBuf::from(result.store_path);

        Ok(ContainerBuilder::new(container_builder_path))
    }

    pub fn read_from_file(path: &CanonicalPath) -> Result<Self, LockedManifestError> {
        let contents = fs::read(path).map_err(LockedManifestError::ReadLockfile)?;
        serde_json::from_slice(&contents).map_err(LockedManifestError::ParseLockfile)
    }
}

impl ToString for LockedManifest {
    fn to_string(&self) -> String {
        serde_json::json!(self).to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct LockedManifestCatalog {
    #[serde(rename = "lockfile-version")]
    pub version: Version<1>,
    /// original manifest that was locked
    pub manifest: TypedManifestCatalog,
    /// locked pacakges
    pub packages: Vec<LockedPackageCatalog>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct LockedPackageCatalog {
    // region: original fields from the service
    // These fields are copied from the generated struct.
    pub attr_path: String,
    pub broken: bool,
    pub derivation: String,
    pub description: String,
    pub license: String,
    pub locked_url: String,
    pub name: String,
    pub pname: String,
    pub rev: String,
    pub rev_count: i64,
    #[cfg_attr(test, proptest(strategy = "crate::utils::proptest_chrono_strategy()"))]
    pub rev_date: chrono::DateTime<chrono::offset::Utc>,
    #[cfg_attr(test, proptest(strategy = "crate::utils::proptest_chrono_strategy()"))]
    pub scrape_date: chrono::DateTime<chrono::offset::Utc>,
    pub stabilities: Vec<String>,
    pub unfree: bool,
    pub version: String,
    // endregion

    // region: converted fields
    pub outputs: BTreeMap<String, String>,
    pub outputs_to_install: Vec<String>,
    // endregion

    // region: added fields
    pub system: System,
    // endregion
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LockedGroup {
    /// name of the group
    name: String,
    /// system this group provides packages for
    system: System,
    /// [CatalogPage] that was selected to fulfill this group
    ///
    /// If resolution of a group provides multiple pages,
    /// a single page is selected based on cross group constraints.
    /// By default this is the latest page that provides packages
    /// for all requested systems.
    page: CatalogPage,
}

impl LockedManifestCatalog {
    /// Convert a locked manifest to a list of installed packages for a given system
    /// in a format shared with the pkgdb based locked manifest.
    pub fn list_packages(&self, system: &System) -> Vec<InstalledPackage> {
        self.packages
            .iter()
            .filter(|package| &package.system == system)
            .map(|package| {
                let priority = self
                    .manifest
                    .install
                    .iter()
                    .find(|(install_id, _)| install_id == &&package.name)
                    .and_then(|(_, descriptor)| descriptor.priority);

                let package = package.clone();

                InstalledPackage {
                    name: package.name,
                    rel_path: package.attr_path,
                    info: PackageInfo {
                        description: Some(package.description),
                        broken: package.broken,
                        license: Some(package.license),
                        pname: package.pname,
                        unfree: package.unfree,
                        version: Some(package.version),
                    },
                    priority,
                }
            })
            .collect()
    }

    /// Produce a lockfile for a given manifest using the catalog service.
    ///
    /// If a seed lockfile is provided, packages that are already locked
    /// will constrain the resolution.
    pub async fn new(
        manifest: &TypedManifestCatalog,
        seed_lockfile: Option<LockedManifestCatalog>,
        client: &impl catalog::ClientTrait,
    ) -> Result<LockedManifestCatalog, catalog::CatalogClientError> {
        let locked_packages = if let Some(ref seed) = seed_lockfile {
            seed.packages
                .iter()
                .filter_map(|package| {
                    let system = &package.system;
                    let manifest = seed.manifest.install.get(&package.name)?;
                    Some(((manifest, system), package))
                })
                .collect()
        } else {
            HashMap::new()
        };

        let groups = Self::collect_package_groups(manifest, locked_packages).collect();

        // lock existing packages

        let resolved = client.resolve(groups).await.unwrap();

        let locked_packages = Self::locked_packages_from_resolution(resolved).collect();

        let lockfile = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: locked_packages,
        };

        Ok(lockfile)
    }

    /// Creates package groups from a flat map of install descriptors
    ///
    /// A group is created for each unique combination of (descriptor.package_group ｘ descriptor.system).
    /// Each group contains a list of package descriptors that belong to that group.
    ///
    /// `locked` is used to provide existing derivations for packages that are already locked,
    /// e.g. by a previous lockfile.
    /// These packages are used to constrain the resolution.
    fn collect_package_groups<'manifest>(
        manifest: &'manifest TypedManifestCatalog,
        locked: HashMap<(&ManifestPackageDescriptor, &System), &LockedPackageCatalog>,
    ) -> impl Iterator<Item = PackageGroup> + 'manifest {
        // Using a btree map to ensure consistent ordering
        let mut map = BTreeMap::new();

        let default_systems = &manifest.options.systems;

        for (install_id, manifest_descriptor) in manifest.install.iter() {
            let resolved_descriptor = PackageDescriptor {
                name: install_id.clone(),
                pkg_path: manifest_descriptor.pkg_path.clone(),
                derivation: None,
                semver: None,
                version: manifest_descriptor.version.clone(),
            };

            let group = manifest_descriptor
                .package_group
                .as_deref()
                .unwrap_or(DEFAULT_GROUP_NAME);

            let descriptor_systems = manifest_descriptor
                .systems
                .as_ref()
                .unwrap_or(default_systems);

            for system in descriptor_systems {
                let resolved_group = map.entry((group, system)).or_insert_with(|| PackageGroup {
                    descriptors: Vec::new(),
                    name: group.to_string(),
                    system: system.clone(),
                });

                let locked_derivation = locked
                    .get(&(manifest_descriptor, system))
                    .map(|p| p.derivation.clone());

                let mut resolved_descriptor = resolved_descriptor.clone();
                resolved_descriptor.derivation = locked_derivation;

                resolved_group.descriptors.push(resolved_descriptor);
            }
        }

        map.into_values()
    }

    /// Convert a resolution results into a list of locked packages
    ///
    /// * Flattens `Group(Page(PackageResolutionInfo+)+)` into `LockedPackageCatalog+`
    /// * Adds a `system` field to each locked package.
    /// * Converts [serde_json::Value] based `outputs` and `outputs_to_install` fields
    /// into [`IndexMap<String, String>`] and [`Vec<String>`] respectively.
    ///
    /// TODO: handle results from multiple pages
    ///       currently there is no api to request packages from specific pages
    /// TODO: handle json value conversion earlier in the shim (or the upstream spec)
    fn locked_packages_from_resolution(
        groups: impl IntoIterator<Item = ResolvedPackageGroup>,
    ) -> impl Iterator<Item = LockedPackageCatalog> {
        groups.into_iter().flat_map(|group| {
            group
                .pages
                .into_iter()
                .take(1)
                .flat_map(|page| page.packages.into_iter())
                .map(move |package| {
                    // unpack package to avoid missing new fields
                    let catalog::PackageResolutionInfo {
                        attr_path,
                        broken,
                        derivation,
                        description,
                        license,
                        locked_url,
                        name,
                        outputs,
                        outputs_to_install,
                        pname,
                        rev,
                        rev_count,
                        rev_date,
                        scrape_date,
                        stabilities,
                        unfree,
                        version,
                    } = package;

                    // todo: server might be able to define otputs as strings directly
                    // todo: comsider adding an intermediate type in the catalog module to handle conversion to strings
                    let outputs = outputs
                        .into_iter()
                        .map(|output| (output.name, output.store_path))
                        .collect();

                    let outputs_to_install = outputs_to_install.into_iter().collect();

                    LockedPackageCatalog {
                        attr_path,
                        broken,
                        derivation,
                        description,
                        license,
                        locked_url,
                        name,
                        outputs,
                        outputs_to_install,
                        pname,
                        rev,
                        rev_count,
                        rev_date,
                        scrape_date,
                        stabilities,
                        unfree,
                        version,
                        system: group.system.clone(),
                    }
                })
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LockedManifestPkgdb(Value);

// region: pkgdb lockfile operations

impl LockedManifestPkgdb {
    /// Use pkgdb to lock a manifest
    ///
    /// `existing_lockfile_path` can be either the global lock or an environment's
    /// lockfile
    pub fn lock_manifest(
        pkgdb: &Path,
        manifest_path: &Path,
        existing_lockfile_path: &CanonicalPath,
        global_manifest_path: &Path,
    ) -> Result<Self, LockedManifestError> {
        let canonical_manifest_path =
            CanonicalPath::new(manifest_path).map_err(LockedManifestError::BadManifestPath)?;

        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd
            .args(["manifest", "lock"])
            .arg("--ga-registry")
            .arg("--global-manifest")
            .arg(global_manifest_path)
            .arg("--manifest")
            .arg(canonical_manifest_path)
            .arg("--lockfile")
            .arg(existing_lockfile_path);

        debug!("locking manifest with command: {}", pkgdb_cmd.display());
        call_pkgdb(pkgdb_cmd)
            .map_err(LockedManifestError::LockManifest)
            .map(Self)
    }

    /// Wrapper around `pkgdb update`
    ///
    /// lockfile_path does not need to exist
    /// TODO: lockfile_path should probably be an Option<CanonicalPath>
    pub fn update_manifest(
        flox: &Flox,
        manifest_path: Option<impl AsRef<Path>>,
        lockfile_path: impl AsRef<Path>,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, LockedManifestError> {
        let lockfile_path = lockfile_path.as_ref();
        let maybe_lockfile = if lockfile_path.exists() {
            debug!("found existing lockfile: {}", lockfile_path.display());
            Some(lockfile_path)
        } else {
            debug!("no existing lockfile found");
            None
        };

        let mut pkgdb_cmd = Command::new(Path::new(&*PKGDB_BIN));
        pkgdb_cmd
            .args(["manifest", "update"])
            .arg("--ga-registry")
            .arg("--global-manifest")
            .arg(global_manifest_path(flox));
        // Optionally add --manifest argument
        if let Some(manifest) = manifest_path {
            pkgdb_cmd.arg("--manifest").arg(manifest.as_ref());
        }
        // Add --lockfile argument if lockfile exists, and parse the old lockfile.
        let old_lockfile = maybe_lockfile
            .map(|lf_path| {
                let canonical_lockfile_path =
                    CanonicalPath::new(lf_path).map_err(LockedManifestError::BadLockfilePath)?;
                pkgdb_cmd.arg("--lockfile").arg(&canonical_lockfile_path);
                LockedManifest::read_from_file(&canonical_lockfile_path)
            })
            .transpose()?;

        // ensure the current lockfile is a Pkgdb lockfile
        let old_lockfile = match old_lockfile {
            Some(LockedManifest::Catalog(_)) => {
                return Err(LockedManifestError::UnsupportedLockfileForUpdate);
            },
            Some(LockedManifest::Pkgdb(locked)) => Some(locked),
            None => None,
        };

        pkgdb_cmd.args(inputs);

        debug!("updating lockfile with command: {}", pkgdb_cmd.display());
        let lockfile: LockedManifestPkgdb =
            LockedManifestPkgdb(call_pkgdb(pkgdb_cmd).map_err(LockedManifestError::UpdateFailed)?);

        Ok(UpdateResult {
            new_lockfile: lockfile,
            old_lockfile,
            store_path: None,
        })
    }

    /// Update global manifest lockfile and write it.
    pub fn update_global_manifest(
        flox: &Flox,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, LockedManifestError> {
        let lockfile_path = global_manifest_lockfile_path(flox);
        let UpdateResult {
            new_lockfile,
            old_lockfile,
            store_path,
        } = Self::update_manifest(flox, None::<PathBuf>, &lockfile_path, inputs)?;

        debug!("writing lockfile to {}", lockfile_path.display());
        std::fs::write(
            lockfile_path,
            serde_json::to_string_pretty(&new_lockfile)
                .map_err(LockedManifestError::SerializeGlobalLockfile)?,
        )
        .map_err(LockedManifestError::WriteGlobalLockfile)?;
        Ok(UpdateResult {
            new_lockfile,
            old_lockfile,
            store_path,
        })
    }

    /// Creates the global lockfile if it doesn't exist and returns its path.
    pub fn ensure_global_lockfile(flox: &Flox) -> Result<PathBuf, LockedManifestError> {
        let global_lockfile_path = global_manifest_lockfile_path(flox);
        if !global_lockfile_path.exists() {
            debug!("Global lockfile does not exist, updating to create one");
            Self::update_global_manifest(flox, vec![])?;
        }
        Ok(global_lockfile_path)
    }

    /// Check the integrity of a lockfile using `pkgdb manifest check`
    pub fn check_lockfile(
        path: &CanonicalPath,
    ) -> Result<Vec<LockfileCheckWarning>, LockedManifestError> {
        let mut pkgdb_cmd = Command::new(Path::new(&*PKGDB_BIN));
        pkgdb_cmd
            .args(["manifest", "check"])
            .arg("--lockfile")
            .arg(path.as_os_str());

        debug!("checking lockfile with command: {}", pkgdb_cmd.display());

        let value = call_pkgdb(pkgdb_cmd).map_err(LockedManifestError::CheckLockfile)?;
        let warnings: Vec<LockfileCheckWarning> =
            serde_json::from_value(value).map_err(LockedManifestError::ParseCheckWarnings)?;

        Ok(warnings)
    }
}

/// An environment (or global) pkgdb lockfile.
///
/// **DEPRECATED**: pkgdb lockfiles are being phased out
/// in favor of catalog lockfiles.
/// Since catalog backed lockfiles are managed within the CLI,
/// [LockedManifestCatalog] provides a typed interface directly,
/// hence there is no catalog equivalent of this type.
///
/// This struct is meant **for reading only**.
///
/// It serves as a typed representation of the lockfile json produced by pkgdb.
/// Parsing of the lockfile is done in [TypedLockedManifest::try_from]
/// and should be as late as possible.
/// Where possible, use the opaque [LockedManifest] instead of this struct
/// to avoid incompatibility issues with the authoritative definition in C++.
///
/// In the optimal case the lockfile schema can be inferred from a common
/// or `pkgdb`-defined schema.
///
/// This struct is used as the format to communicate with pkgdb.
/// Many pkgdb commands will need to pass some of the information in the
/// lockfile through to Rust.
///
/// And some commands (i.e. `list`) will need to read lockfiles
/// to get information about the environment without having to call `pkgdb`.
///
/// Although we could selectively pass fields through,
/// I'm hoping it will be easier to parse the entirety of the lockfile in Rust,
/// rather than defining a separate set of fields for each different pkgdb
/// command.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct TypedLockedManifestPkgdb {
    #[serde(rename = "lockfile-version")]
    lockfile_version: Version<0>,
    packages: BTreeMap<System, BTreeMap<String, Option<LockedPackagePkgdb>>>,
    registry: Registry,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct LockedPackagePkgdb {
    info: PackageInfo,
    #[serde(rename = "attr-path")]
    abs_path: Vec<String>,
    priority: usize,
}

impl LockedPackagePkgdb {
    pub fn rel_path(&self) -> String {
        self.abs_path
            .iter()
            .skip(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(".")
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PackageInfo {
    pub description: Option<String>,
    pub broken: bool,
    pub license: Option<String>,
    pub pname: String,
    pub unfree: bool,
    pub version: Option<String>,
}

impl TryFrom<LockedManifestPkgdb> for TypedLockedManifestPkgdb {
    type Error = LockedManifestError;

    fn try_from(value: LockedManifestPkgdb) -> Result<Self, Self::Error> {
        serde_json::from_value(value.0).map_err(LockedManifestError::ParseLockedManifest)
    }
}

impl TypedLockedManifestPkgdb {
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// List all packages in the locked manifest for a given system
    pub fn list_packages(&self, system: &System) -> Vec<InstalledPackage> {
        let mut packages = vec![];
        if let Some(system_packages) = self.packages.get(system) {
            for (name, locked_package) in system_packages {
                if let Some(locked_package) = locked_package {
                    packages.push(InstalledPackage {
                        name: name.clone(),
                        rel_path: locked_package.rel_path(),
                        info: locked_package.info.clone(),
                        priority: Some(locked_package.priority),
                    });
                };
            }
        }
        packages
    }
}

// endregion

// TODO: consider dropping this in favor of mapping to [LockedPackageCatalog]?
/// A locked package with additionally derived attributes
pub struct InstalledPackage {
    pub name: String,
    pub rel_path: String,
    pub info: PackageInfo,
    pub priority: Option<usize>,
}

#[derive(Debug, Error)]
pub enum LockedManifestError {
    #[error("failed to lock manifest")]
    LockManifest(#[source] CallPkgDbError),
    #[error("failed to check lockfile")]
    CheckLockfile(#[source] CallPkgDbError),
    #[error("failed to build environment")]
    BuildEnv(#[source] CallPkgDbError),
    #[error("failed to parse check warnings")]
    ParseCheckWarnings(#[source] serde_json::Error),
    #[error("package is unsupported for this sytem")]
    UnsupportedPackageWithDocLink(#[source] CallPkgDbError),
    #[error("failed to build container builder")]
    CallContainerBuilder(#[source] std::io::Error),
    #[error("failed to write container builder to sink")]
    WriteContainer(#[source] std::io::Error),
    #[error("failed to parse buildenv output")]
    ParseBuildEnvOutput(#[source] serde_json::Error),
    #[error("failed to update environment")]
    UpdateFailed(#[source] CallPkgDbError),
    #[error(transparent)]
    BadManifestPath(CanonicalizeError),
    #[error(transparent)]
    BadLockfilePath(CanonicalizeError),
    #[error("could not open manifest file")]
    ReadLockfile(#[source] std::io::Error),
    /// when parsing the contents of a lockfile into a [LockedManifest]
    #[error("could not parse lockfile")]
    ParseLockfile(#[source] serde_json::Error),
    /// when parsing a [LockedManifest] into a [TypedLockedManifest]
    #[error("failed to parse contents of locked manifest")]
    ParseLockedManifest(#[source] serde_json::Error),
    #[error("could not serialize global lockfile")]
    SerializeGlobalLockfile(#[source] serde_json::Error),
    #[error("could not write global lockfile")]
    WriteGlobalLockfile(#[source] std::io::Error),

    #[error("Catalog lockfile does not support update")]
    UnsupportedLockfileForUpdate,
}

/// A warning produced by `pkgdb manifest check`
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LockfileCheckWarning {
    pub package: String,
    pub message: String,
}

#[cfg(any(test, feature = "test"))]
mod tests {
    use std::collections::HashMap;

    use catalog_api_v1::types::Output;
    use indoc::indoc;
    use once_cell::sync::Lazy;
    use pretty_assertions::assert_eq;

    use self::catalog::PackageResolutionInfo;
    use super::*;
    use crate::models::manifest::{RawManifest, TypedManifest};

    /// Validate that the parser for the locked manifest can handle null values
    /// for the `version`, `license`, and `description` fields.
    #[test]
    fn locked_package_tolerates_null_values() {
        let locked_packages =
            serde_json::from_value::<HashMap<String, LockedPackagePkgdb>>(serde_json::json!({
                    "complete": {
                        "info": {
                            "description": "A package",
                            "broken": false,
                            "license": "MIT",
                            "pname": "package1",
                            "unfree": false,
                            "version": "1.0.0"
                        },
                        "attr-path": ["package1"],
                        "priority": 0
                    },
                    "missing_version": {
                        "info": {
                            "description": "Another package",
                            "broken": false,
                            "license": "MIT",
                            "pname": "package2",
                            "unfree": false,
                            "version": null
                        },
                        "attr-path": ["package2"],
                        "priority": 0
                    },
                    "missing_license": {
                        "info": {
                            "description": "Another package",
                            "broken": false,
                            "license": null,
                            "pname": "package3",
                            "unfree": false,
                            "version": "1.0.0"
                        },
                        "attr-path": ["package3"],
                        "priority": 0
                    },
                    "missing_description": {
                        "info": {
                            "description": null,
                            "broken": false,
                            "license": "MIT",
                            "pname": "package4",
                            "unfree": false,
                            "version": "1.0.0"
                        },
                        "attr-path": ["package4"],
                        "priority": 0
                    },
            }))
            .unwrap();

        assert_eq!(
            locked_packages["complete"].info.version.as_deref(),
            Some("1.0.0")
        );
        assert_eq!(
            locked_packages["complete"].info.license.as_deref(),
            Some("MIT")
        );
        assert_eq!(
            locked_packages["complete"].info.description.as_deref(),
            Some("A package")
        );

        assert_eq!(
            locked_packages["missing_version"].info.version.as_deref(),
            None
        );
        assert_eq!(
            locked_packages["missing_license"].info.license.as_deref(),
            None
        );
        assert_eq!(
            locked_packages["missing_description"]
                .info
                .description
                .as_deref(),
            None
        );
    }

    static TEST_RAW_MANIFEST: Lazy<RawManifest> = Lazy::new(|| {
        indoc! {r#"
          version = 1

          [install]
          hello.pkg-path = "hello"
          hello.package-group = "group"

          [options]
          systems = ["system"]
        "#}
        .parse()
        .unwrap()
    });

    static TEST_TYPED_MANIFEST: Lazy<TypedManifestCatalog> = Lazy::new(|| {
        let typed = TEST_RAW_MANIFEST.to_typed().unwrap();
        match typed {
            TypedManifest::Catalog(manifest) => *manifest,
            _ => panic!("Expected a catalog manifest"),
        }
    });

    static TEST_RESOLUTION_PARAMS: Lazy<Vec<PackageGroup>> = Lazy::new(|| {
        vec![PackageGroup {
            name: "group".to_string(),
            system: "system".to_string(),
            descriptors: vec![PackageDescriptor {
                name: "hello".to_string(),
                pkg_path: "hello".to_string(),
                derivation: None,
                semver: None,
                version: None,
            }],
        }]
    });

    static TEST_RESOLUTION_RESPONSE: Lazy<Vec<ResolvedPackageGroup>> = Lazy::new(|| {
        vec![ResolvedPackageGroup {
            system: "system".to_string(),
            pages: vec![CatalogPage {
                page: 1,
                url: "url".to_string(),
                packages: vec![PackageResolutionInfo {
                    attr_path: "hello".to_string(),
                    broken: false,
                    derivation: "derivation".to_string(),
                    description: "description".to_string(),
                    license: "license".to_string(),
                    locked_url: "locked_url".to_string(),
                    name: "name".to_string(),
                    outputs: vec![Output {
                        name: "name".to_string(),
                        store_path: "store_path".to_string(),
                    }],
                    outputs_to_install: vec!["name".to_string()],
                    pname: "pname".to_string(),
                    rev: "rev".to_string(),
                    rev_count: 1,
                    rev_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                        .unwrap()
                        .with_timezone(&chrono::offset::Utc),
                    scrape_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                        .unwrap()
                        .with_timezone(&chrono::offset::Utc),
                    stabilities: vec!["stability".to_string()],
                    unfree: false,
                    version: "version".to_string(),
                }],
            }],
            name: "group".to_string(),
        }]
    });

    static TEST_LOCKED_MANIFEST: Lazy<LockedManifest> = Lazy::new(|| {
        LockedManifest::Catalog(LockedManifestCatalog {
            version: Version::<1>,
            manifest: TEST_TYPED_MANIFEST.clone(),
            packages: vec![LockedPackageCatalog {
                attr_path: "hello".to_string(),
                broken: false,
                derivation: "derivation".to_string(),
                description: "description".to_string(),
                license: "license".to_string(),
                locked_url: "locked_url".to_string(),
                name: "name".to_string(),
                outputs: vec![("name".to_string(), "store_path".to_string())]
                    .into_iter()
                    .collect(),
                outputs_to_install: vec!["name".to_string()],
                pname: "pname".to_string(),
                rev: "rev".to_string(),
                rev_count: 1,
                rev_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::offset::Utc),
                scrape_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                    .unwrap()
                    .with_timezone(&chrono::offset::Utc),
                stabilities: vec!["stability".to_string()],
                unfree: false,
                version: "version".to_string(),
                system: "system".to_string(),
            }],
        })
    });

    #[test]
    fn resolve_params() {
        let manifest = &*TEST_TYPED_MANIFEST;

        let params = LockedManifestCatalog::collect_package_groups(manifest, Default::default())
            .collect::<Vec<_>>();
        assert_eq!(&params, &*TEST_RESOLUTION_PARAMS);
    }

    #[tokio::test]
    async fn test_locking_1() {
        let manifest = &*TEST_TYPED_MANIFEST;

        let mut client = catalog::MockClient::new(None::<String>).unwrap();
        client.push_resolve_response(TEST_RESOLUTION_RESPONSE.clone());

        let locked_manifest = LockedManifestCatalog::new(manifest, None, &client)
            .await
            .unwrap();
        assert_eq!(
            &LockedManifest::Catalog(locked_manifest),
            &*TEST_LOCKED_MANIFEST
        );
    }
}
