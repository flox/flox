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
use super::manifest::{
    ManifestPackageDescriptor,
    TypedManifestCatalog,
    DEFAULT_GROUP_NAME,
    DEFAULT_PRIORITY,
};
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
    PackageResolutionInfo,
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
    pub description: Option<String>,
    pub install_id: String,
    pub license: Option<String>,
    pub locked_url: String,
    pub name: String,
    pub pname: String,
    pub rev: String,
    pub rev_count: i64,
    #[cfg_attr(test, proptest(strategy = "crate::utils::proptest_chrono_strategy()"))]
    pub rev_date: chrono::DateTime<chrono::offset::Utc>,
    #[cfg_attr(test, proptest(strategy = "crate::utils::proptest_chrono_strategy()"))]
    pub scrape_date: chrono::DateTime<chrono::offset::Utc>,
    pub stabilities: Option<Vec<String>>,
    pub unfree: Option<bool>,
    pub version: String,
    pub outputs_to_install: Option<Vec<String>>,
    // endregion

    // region: converted fields
    pub outputs: Option<BTreeMap<String, String>>,
    // endregion

    // region: added fields
    pub system: System,
    pub group: String,
    pub priority: usize,
    pub optional: bool,
    // endregion
}

impl LockedPackageCatalog {
    /// Construct a [LockedPackageCatalog] from a [ManifestPackageDescriptor],
    /// the resolved [catalog::PackageResolutionInfo], and corresponding [System].
    ///
    /// There may be more validation/parsing we could do here in the future.
    pub fn from_parts(
        package: catalog::PackageResolutionInfo,
        descriptor: ManifestPackageDescriptor,
        system: System,
    ) -> Self {
        // unpack package to avoid missing new fields
        let catalog::PackageResolutionInfo {
            attr_path,
            broken,
            derivation,
            description,
            install_id,
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

        let outputs = outputs.map(|outputs| {
            outputs
                .into_iter()
                .map(|output| (output.name, output.store_path))
                .collect()
        });

        let priority = descriptor.priority.unwrap_or(DEFAULT_PRIORITY);
        let group = descriptor
            .pkg_group
            .as_deref()
            .unwrap_or(DEFAULT_GROUP_NAME)
            .to_string();
        let optional = descriptor.optional;

        LockedPackageCatalog {
            attr_path,
            broken,
            derivation,
            description,
            install_id,
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
            system,
            priority,
            group,
            optional,
        }
    }
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
            .cloned()
            .map(|package| InstalledPackage {
                install_id: package.install_id,
                rel_path: package.attr_path,
                info: PackageInfo {
                    description: package.description,
                    broken: package.broken,
                    license: package.license,
                    pname: package.pname,
                    unfree: package.unfree,
                    version: Some(package.version),
                },
                priority: Some(package.priority),
            })
            .collect()
    }

    /// Produce a lockfile for a given manifest using the catalog service.
    ///
    /// If a seed lockfile is provided, packages that are already locked
    /// will constrain the resolution.
    pub async fn lock_manifest(
        manifest: &TypedManifestCatalog,
        seed_lockfile: Option<&LockedManifestCatalog>,
        client: &impl catalog::ClientTrait,
    ) -> Result<LockedManifestCatalog, LockedManifestError> {
        let groups = Self::collect_package_groups(manifest, seed_lockfile).collect();

        // lock existing packages

        let resolved = client
            .resolve(groups)
            .await
            .map_err(LockedManifestError::CatalogResolve)?;

        let locked_packages = Self::locked_packages_from_resolution(manifest, resolved)?.collect();

        let lockfile = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: locked_packages,
        };

        Ok(lockfile)
    }

    /// Transform a lockfile into a mapping  that is easier to query:
    /// Lockfile -> { (package, system): locked package }
    fn make_seed_mapping(
        seed: &LockedManifestCatalog,
    ) -> HashMap<(&ManifestPackageDescriptor, &System), &LockedPackageCatalog> {
        seed.packages
            .iter()
            .filter_map(|package| {
                let system = &package.system;
                let manifest = seed.manifest.install.get(&package.install_id)?;
                Some(((manifest, system), package))
            })
            .collect()
    }

    /// Creates package groups from a flat map of install descriptors
    ///
    /// A group is created for each unique combination of (descriptor.package_group ｘ descriptor.systems).
    /// If descriptor.systems is None, a group with default_system is created for each package_group.
    /// Each group contains a list of package descriptors that belong to that group.
    ///
    /// `seed_lockfile` is used to provide existing derivations for packages that are already locked,
    /// e.g. by a previous lockfile.
    /// These packages are used to constrain the resolution.
    /// If a package in `manifest` does not have a corresponding package in `seed_lockfile`,
    /// that package will be unconstrained, allowing a first install.
    fn collect_package_groups(
        manifest: &TypedManifestCatalog,
        seed_lockfile: Option<&LockedManifestCatalog>,
    ) -> impl Iterator<Item = PackageGroup> {
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        // Using a btree map to ensure consistent ordering
        let mut map = BTreeMap::new();

        let default_systems = vec![
            "aarch64-darwin".to_string(),
            "aarch64-linux".to_string(),
            "x86_64-darwin".to_string(),
            "x86_64-linux".to_string(),
        ];
        let manifest_systems = manifest
            .options
            .systems
            .as_ref()
            .unwrap_or(&default_systems);

        for (install_id, manifest_descriptor) in manifest.install.iter() {
            let resolved_descriptor = PackageDescriptor {
                install_id: install_id.clone(),
                attr_path: manifest_descriptor.pkg_path.clone(),
                derivation: None,
                version: manifest_descriptor.version.clone(),
                allow_pre_releases: manifest.options.semver.allow_pre_releases,
            };

            let group = manifest_descriptor
                .pkg_group
                .as_deref()
                .unwrap_or(DEFAULT_GROUP_NAME);

            let descriptor_systems = manifest_descriptor
                .systems
                .as_ref()
                .unwrap_or(manifest_systems);

            for system in descriptor_systems {
                let resolved_group = map
                    .entry((group.to_string(), system.clone()))
                    .or_insert_with(|| PackageGroup {
                        descriptors: Vec::new(),
                        name: group.to_string(),
                        system: system.clone(),
                    });

                // If the package was just added to the manifest, it will be missing in the seed,
                // which is derived from the _previous_ lockfile.
                // In this case, the derivation will be None, and the package will be unconstrained.
                let locked_derivation = seed_locked_packages
                    .get(&(manifest_descriptor, &system.to_string()))
                    .map(|p| p.derivation.clone());

                let mut resolved_descriptor = resolved_descriptor.clone();
                resolved_descriptor.derivation = locked_derivation;

                resolved_group.descriptors.push(resolved_descriptor);
            }
        }

        map.into_values()
    }

    /// Convert resolution results into a list of locked packages
    ///
    /// * Flattens `Group(Page(PackageResolutionInfo+)+)` into `LockedPackageCatalog+`
    /// * Adds a `system` field to each locked package.
    /// * Converts [serde_json::Value] based `outputs` and `outputs_to_install` fields
    /// into [`IndexMap<String, String>`] and [`Vec<String>`] respectively.
    ///
    /// TODO: handle results from multiple pages
    ///       currently there is no api to request packages from specific pages
    /// TODO: handle json value conversion earlier in the shim (or the upstream spec)
    fn locked_packages_from_resolution<'manifest>(
        manifest: &'manifest TypedManifestCatalog,
        groups: impl IntoIterator<Item = ResolvedPackageGroup> + 'manifest,
    ) -> Result<impl Iterator<Item = LockedPackageCatalog> + 'manifest, LockedManifestError> {
        // For each group, extract the first page and its system.
        // Error if the first page doesn't contain any packages.
        let first_pages: Vec<(Vec<PackageResolutionInfo>, String)> = groups
            .into_iter()
            .map(|mut group| {
                group
                    .pages
                    .first_mut()
                    .and_then(|page| {
                        std::mem::take(&mut page.packages)
                            .map(|packages| (packages, group.system.clone()))
                    })
                    .ok_or_else(|| {
                        LockedManifestError::NoPackagesOnFirstPage(group.name, group.system)
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Flatten packages from all the groups into a single iterator
        let infos = first_pages.into_iter().flat_map(|(packages, system)| {
            packages
                .into_iter()
                .map(move |package| (package, system.clone()))
        });

        Ok(infos.filter_map(|(package, system)| {
            let Some(descriptor) = manifest.install.get(&package.install_id).cloned() else {
                debug!(
                    "Package {} is not in the manifest, skipping",
                    package.install_id
                );
                return None;
            };

            Some(LockedPackageCatalog::from_parts(
                package, descriptor, system,
            ))
        }))
    }

    /// Filter out packages from the locked manifest by install_id or group
    ///
    /// This is used to create a seed lockfile to upgrade a subset of packages,
    /// as packages that are not in the seed lockfile will be re-resolved unconstrained.
    pub(crate) fn unlock_packages_by_group_or_iid(
        &mut self,
        groups_or_iids: &[String],
    ) -> &mut Self {
        self.packages.retain(|package| {
            !groups_or_iids.contains(&package.install_id)
                && !groups_or_iids.contains(&package.group)
        });

        self
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
    pub unfree: Option<bool>,
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
            for (install_id, locked_package) in system_packages {
                if let Some(locked_package) = locked_package {
                    packages.push(InstalledPackage {
                        install_id: install_id.clone(),
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
    pub install_id: String,
    pub rel_path: String,
    pub info: PackageInfo,
    pub priority: Option<usize>,
}

#[derive(Debug, Error)]
pub enum LockedManifestError {
    #[error("failed to resolve packages")]
    CatalogResolve(#[from] catalog::ResolveError),
    #[error("didn't find packages on the first page of the group {0} for system {1}")]
    NoPackagesOnFirstPage(String, String),
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
    #[error("could not open lockfile")]
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

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::vec;

    use catalog_api_v1::types::Output;
    use indoc::indoc;
    use once_cell::sync::Lazy;
    use pretty_assertions::assert_eq;

    use self::catalog::PackageResolutionInfo;
    use super::*;
    use crate::models::manifest::{self, RawManifest, TypedManifest};

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
          hello_install_id.pkg-path = "hello"
          hello_install_id.pkg-group = "group"

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
                install_id: "hello_install_id".to_string(),
                attr_path: "hello".to_string(),
                derivation: None,
                version: None,
                allow_pre_releases: None,
            }],
        }]
    });

    static TEST_RESOLUTION_RESPONSE: Lazy<Vec<ResolvedPackageGroup>> = Lazy::new(|| {
        vec![ResolvedPackageGroup {
            system: "system".to_string(),
            pages: vec![CatalogPage {
                page: 1,
                url: "url".to_string(),
                packages: Some(vec![PackageResolutionInfo {
                    attr_path: "hello".to_string(),
                    broken: false,
                    derivation: "derivation".to_string(),
                    description: Some("description".to_string()),
                    install_id: "hello_install_id".to_string(),
                    license: Some("license".to_string()),
                    locked_url: "locked_url".to_string(),
                    name: "hello".to_string(),
                    outputs: Some(vec![Output {
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
                    scrape_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                        .unwrap()
                        .with_timezone(&chrono::offset::Utc),
                    stabilities: Some(vec!["stability".to_string()]),
                    unfree: Some(false),
                    version: "version".to_string(),
                }]),
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
                description: Some("description".to_string()),
                install_id: "hello_install_id".to_string(),
                license: Some("license".to_string()),
                locked_url: "locked_url".to_string(),
                name: "hello".to_string(),
                outputs: Some(
                    vec![("name".to_string(), "store_path".to_string())]
                        .into_iter()
                        .collect(),
                ),
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
                system: "system".to_string(),
                group: "group".to_string(),
                priority: 5,
                optional: false,
            }],
        })
    });

    pub(crate) fn fake_package(
        name: &str,
        group: Option<&str>,
    ) -> (String, ManifestPackageDescriptor, LockedPackageCatalog) {
        let install_id = format!("{}_install_id", name);

        let descriptor = ManifestPackageDescriptor {
            pkg_path: name.to_string(),
            pkg_group: group.map(|s| s.to_string()),
            systems: Some(vec!["system".to_string()]),
            version: None,
            priority: None,
            optional: false,
        };

        let locked = LockedPackageCatalog {
            attr_path: name.to_string(),
            broken: false,
            derivation: "derivation".to_string(),
            description: None,
            install_id: install_id.clone(),
            license: None,
            locked_url: "".to_string(),
            name: name.to_string(),
            outputs: None,
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
            system: "system".to_string(),
            group: group.unwrap_or(DEFAULT_GROUP_NAME).to_string(),
            priority: 5,
            optional: false,
        };
        (install_id, descriptor, locked)
    }

    #[test]
    fn make_params_smoke() {
        let manifest = &*TEST_TYPED_MANIFEST;

        let params =
            LockedManifestCatalog::collect_package_groups(manifest, None).collect::<Vec<_>>();
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
            systems = ["system1", "system2"]
        "#};
        let manifest = toml::from_str(manifest_str).unwrap();

        let expected_params = vec![
            PackageGroup {
                name: DEFAULT_GROUP_NAME.to_string(),
                system: "system1".to_string(),
                descriptors: vec![
                    PackageDescriptor {
                        allow_pre_releases: None,
                        attr_path: "emacs".to_string(),
                        derivation: None,
                        install_id: "emacs".to_string(),
                        version: None,
                    },
                    PackageDescriptor {
                        allow_pre_releases: None,
                        attr_path: "vim".to_string(),
                        derivation: None,
                        install_id: "vim".to_string(),
                        version: None,
                    },
                ],
            },
            PackageGroup {
                name: DEFAULT_GROUP_NAME.to_string(),
                system: "system2".to_string(),
                descriptors: vec![
                    PackageDescriptor {
                        allow_pre_releases: None,
                        attr_path: "emacs".to_string(),
                        derivation: None,
                        install_id: "emacs".to_string(),
                        version: None,
                    },
                    PackageDescriptor {
                        allow_pre_releases: None,
                        attr_path: "vim".to_string(),
                        derivation: None,
                        install_id: "vim".to_string(),
                        version: None,
                    },
                ],
            },
        ];

        let actual_params =
            LockedManifestCatalog::collect_package_groups(&manifest, None).collect::<Vec<_>>();

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
            emacs.systems = ["system1"]

            [options]
            systems = ["system1", "system2"]
        "#};
        let manifest = toml::from_str(manifest_str).unwrap();

        let expected_params = vec![
            PackageGroup {
                name: DEFAULT_GROUP_NAME.to_string(),
                system: "system1".to_string(),
                descriptors: vec![
                    PackageDescriptor {
                        allow_pre_releases: None,
                        attr_path: "emacs".to_string(),
                        install_id: "emacs".to_string(),
                        derivation: None,
                        version: None,
                    },
                    PackageDescriptor {
                        allow_pre_releases: None,
                        attr_path: "vim".to_string(),
                        derivation: None,
                        install_id: "vim".to_string(),
                        version: None,
                    },
                ],
            },
            PackageGroup {
                name: DEFAULT_GROUP_NAME.to_string(),
                system: "system2".to_string(),
                descriptors: vec![PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                }],
            },
        ];

        let actual_params =
            LockedManifestCatalog::collect_package_groups(&manifest, None).collect::<Vec<_>>();

        assert_eq!(actual_params, expected_params);
    }

    /// If a package specifies a system not in `options.systems`,
    /// use those instead.
    #[test]
    fn make_params_override_systems() {
        let manifest_str = indoc! {r#"
            version = 1

            [install]
            vim.pkg-path = "vim"
            emacs.pkg-path = "emacs"
            emacs.systems = ["system2"]

            [options]
            systems = ["system1",]
        "#};
        let manifest = toml::from_str(manifest_str).unwrap();

        let expected_params = vec![
            PackageGroup {
                name: DEFAULT_GROUP_NAME.to_string(),
                system: "system1".to_string(),
                descriptors: vec![PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                }],
            },
            PackageGroup {
                name: DEFAULT_GROUP_NAME.to_string(),
                system: "system2".to_string(),
                descriptors: vec![PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "emacs".to_string(),
                    derivation: None,
                    install_id: "emacs".to_string(),
                    version: None,
                }],
            },
        ];

        let actual_params =
            LockedManifestCatalog::collect_package_groups(&manifest, None).collect::<Vec<_>>();

        assert_eq!(actual_params, expected_params);
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
            systems = ["system"]
        "#};

        let manifest = toml::from_str(manifest_str).unwrap();

        let expected_params = vec![
            PackageGroup {
                name: "group1".to_string(),
                system: "system".to_string(),
                descriptors: vec![PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                }],
            },
            PackageGroup {
                name: "group2".to_string(),
                system: "system".to_string(),
                descriptors: vec![PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "emacs".to_string(),
                    derivation: None,
                    install_id: "emacs".to_string(),
                    version: None,
                }],
            },
        ];

        let actual_params =
            LockedManifestCatalog::collect_package_groups(&manifest, None).collect::<Vec<_>>();

        assert_eq!(actual_params, expected_params);
    }

    /// If a seed mapping is provided, use the derivations from the seed where possible
    #[test]
    fn make_params_seeded() {
        let mut manifest = TEST_TYPED_MANIFEST.clone();

        // Add a package to the manifest that is not already locked
        manifest
            .install
            .insert("unlocked".to_string(), ManifestPackageDescriptor {
                pkg_path: "unlocked".to_string(),
                pkg_group: Some("group".to_string()),
                systems: None,
                version: None,
                priority: None,
                optional: false,
            });

        let LockedManifest::Catalog(seed) = &*TEST_LOCKED_MANIFEST else {
            panic!("Expected a catalog lockfile");
        };

        let actual_params = LockedManifestCatalog::collect_package_groups(&manifest, Some(seed))
            .collect::<Vec<_>>();

        let expected_params = vec![PackageGroup {
            name: "group".to_string(),
            system: "system".to_string(),
            descriptors: vec![
                // 'hello' was already locked, so it should have a derivation
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "hello".to_string(),
                    derivation: Some("derivation".to_string()),
                    install_id: "hello_install_id".to_string(),
                    version: None,
                },
                // The unlocked package should not have a derivation
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "unlocked".to_string(),
                    derivation: None,
                    install_id: "unlocked".to_string(),
                    version: None,
                },
            ],
        }];

        assert_eq!(actual_params, expected_params);
    }

    #[test]
    fn ungroup_response() {
        let groups = vec![ResolvedPackageGroup {
            system: "system".to_string(),
            pages: vec![CatalogPage {
                page: 1,
                url: "url".to_string(),
                packages: Some(vec![PackageResolutionInfo {
                    attr_path: "hello".to_string(),
                    broken: false,
                    derivation: "derivation".to_string(),
                    description: Some("description".to_string()),
                    install_id: "hello_install_id".to_string(),
                    license: Some("license".to_string()),
                    locked_url: "locked_url".to_string(),
                    name: "hello".to_string(),
                    outputs: Some(vec![Output {
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
                    scrape_date: chrono::DateTime::parse_from_rfc3339("2021-08-31T00:00:00Z")
                        .unwrap()
                        .with_timezone(&chrono::offset::Utc),
                    stabilities: Some(vec!["stability".to_string()]),
                    unfree: Some(false),
                    version: "version".to_string(),
                }]),
            }],
            name: "group".to_string(),
        }];

        let manifest = &*TEST_TYPED_MANIFEST;

        let locked_packages =
            LockedManifestCatalog::locked_packages_from_resolution(manifest, groups.clone())
                .unwrap()
                .collect::<Vec<_>>();

        assert_eq!(locked_packages.len(), 1);
        assert_eq!(
            &locked_packages[0],
            &LockedPackageCatalog::from_parts(
                groups[0].pages[0].packages.as_ref().unwrap()[0].clone(),
                manifest
                    .install
                    .get(&groups[0].pages[0].packages.as_ref().unwrap()[0].install_id)
                    .unwrap()
                    .clone(),
                groups[0].system.clone()
            )
        );
    }

    /// unlocking by iid should remove only the package with that iid
    #[test]
    fn unlock_by_iid() {
        let mut manifest = manifest::test::empty_catalog_manifest();
        let (foo_iid, foo_descriptor, foo_locked) = fake_package("foo", None);
        let (bar_iid, bar_descriptor, bar_locked) = fake_package("bar", None);
        manifest.install.insert(foo_iid.clone(), foo_descriptor);
        manifest.install.insert(bar_iid.clone(), bar_descriptor);
        let mut lockfile = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.clone(), bar_locked.clone()],
        };

        lockfile.unlock_packages_by_group_or_iid(&[foo_iid.clone()]);

        assert_eq!(lockfile.packages, vec![bar_locked]);
    }

    /// Unlocking by group should remove all packages in that group
    #[test]
    fn unlock_by_group() {
        let mut manifest = manifest::test::empty_catalog_manifest();
        let (foo_iid, foo_descriptor, foo_locked) = fake_package("foo", Some("group"));
        let (bar_iid, bar_descriptor, bar_locked) = fake_package("bar", Some("group"));
        manifest.install.insert(foo_iid.clone(), foo_descriptor);
        manifest.install.insert(bar_iid.clone(), bar_descriptor);
        let mut lockfile = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.clone(), bar_locked.clone()],
        };

        lockfile.unlock_packages_by_group_or_iid(&["group".to_string()]);

        assert_eq!(lockfile.packages, vec![]);
    }

    /// If an unlocked iid is also used as a group, remove both the group
    /// and the package
    #[test]
    fn unlock_by_iid_and_group() {
        let mut manifest = manifest::test::empty_catalog_manifest();
        let (foo_iid, foo_descriptor, foo_locked) = fake_package("foo", Some("foo_install_id"));
        let (bar_iid, bar_descriptor, bar_locked) = fake_package("bar", Some("foo_install_id"));
        manifest.install.insert(foo_iid.clone(), foo_descriptor);
        manifest.install.insert(bar_iid.clone(), bar_descriptor);
        let mut lockfile = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.clone(), bar_locked.clone()],
        };

        lockfile.unlock_packages_by_group_or_iid(&[foo_iid.clone()]);

        assert_eq!(lockfile.packages, vec![]);
    }

    #[test]
    fn unlock_by_iid_noop_if_already_unlocked() {
        let LockedManifest::Catalog(mut seed) = TEST_LOCKED_MANIFEST.clone() else {
            panic!("Expected a catalog lockfile");
        };

        // If the package is not in the seed, the lockfile should be unchanged
        let expected = seed.packages.clone();

        seed.unlock_packages_by_group_or_iid(&["not in here".to_string()]);

        assert_eq!(seed.packages, expected,);
    }

    #[tokio::test]
    async fn test_locking_1() {
        let manifest = &*TEST_TYPED_MANIFEST;

        let mut client = catalog::MockClient::new(None::<String>).unwrap();
        client.push_resolve_response(TEST_RESOLUTION_RESPONSE.clone());

        let locked_manifest = LockedManifestCatalog::lock_manifest(manifest, None, &client)
            .await
            .unwrap();
        assert_eq!(
            &LockedManifest::Catalog(locked_manifest),
            &*TEST_LOCKED_MANIFEST
        );
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
        let manifest: TypedManifestCatalog = toml::from_str(manifest_str).unwrap();
        let package_groups: Vec<_> =
            LockedManifestCatalog::collect_package_groups(&manifest, None).collect();

        assert_eq!(package_groups.len(), 4);
        let expected_systems = [
            "aarch64-darwin".to_string(),
            "aarch64-linux".to_string(),
            "x86_64-darwin".to_string(),
            "x86_64-linux".to_string(),
        ];

        for group in package_groups {
            assert!(expected_systems.contains(&group.system))
        }
    }
}
