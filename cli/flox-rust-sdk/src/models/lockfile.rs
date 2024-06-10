use catalog_api_v1::types::SystemEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type FlakeRef = Value;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

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
    pub broken: Option<bool>,
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
    pub outputs: BTreeMap<String, String>,
    // endregion

    // region: added fields
    pub system: System, // FIXME: this is an enum in the generated code, can't derive Arbitrary there
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
            system,
        } = package;

        let outputs = outputs
            .into_iter()
            .map(|output| (output.name, output.store_path))
            .collect::<BTreeMap<_, _>>();

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
            system: system.to_string(),
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
        let groups = Self::collect_package_groups(manifest, seed_lockfile)?;
        let (already_locked_packages, groups_to_lock) =
            Self::split_fully_locked_groups(groups, seed_lockfile);

        if groups_to_lock.is_empty() {
            debug!("All packages are already locked, skipping resolution");
            return Ok(LockedManifestCatalog {
                version: Version::<1>,
                manifest: manifest.clone(),
                packages: already_locked_packages,
            });
        }

        // lock packages
        let resolved = client
            .resolve(groups_to_lock)
            .await
            .map_err(LockedManifestError::CatalogResolve)?;

        // unpack locked packages from response
        let locked_packages = Self::locked_packages_from_resolution(manifest, resolved)?.collect();

        let lockfile = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: [already_locked_packages, locked_packages].concat(),
        };

        Ok(lockfile)
    }

    /// Transform a lockfile into a mapping that is easier to query:
    /// Lockfile -> { (install_id, system): (package_descriptor, locked_package) }
    fn make_seed_mapping(
        seed: &LockedManifestCatalog,
    ) -> HashMap<(&String, &System), (&ManifestPackageDescriptor, &LockedPackageCatalog)> {
        seed.packages
            .iter()
            .filter_map(|locked| {
                let system = &locked.system;
                let install_id = &locked.install_id;
                let descriptor = seed.manifest.install.get(&locked.install_id)?;
                Some(((install_id, system), (descriptor, locked)))
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
    ) -> Result<impl Iterator<Item = PackageGroup>, LockedManifestError> {
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        // Using a btree map to ensure consistent ordering
        let mut map = BTreeMap::new();

        let default_systems = [
            "aarch64-darwin".to_string(),
            "aarch64-linux".to_string(),
            "x86_64-darwin".to_string(),
            "x86_64-linux".to_string(),
        ];
        let manifest_systems = manifest.options.systems.as_deref();

        let maybe_licenses = if manifest.options.allow.licenses.is_empty() {
            None
        } else {
            Some(manifest.options.allow.licenses.clone())
        };

        for (install_id, manifest_descriptor) in manifest.install.iter() {
            let resolved_descriptor_base = PackageDescriptor {
                install_id: install_id.clone(),
                attr_path: manifest_descriptor.pkg_path.clone(),
                derivation: None,
                version: manifest_descriptor.version.clone(),
                allow_pre_releases: manifest.options.semver.allow_pre_releases,
                allow_broken: manifest.options.allow.broken,
                allow_unfree: manifest.options.allow.unfree,
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

            let systems = manifest_descriptor
                .systems
                .as_deref()
                .or(manifest_systems)
                .unwrap_or(&default_systems)
                .iter()
                .map(|s| {
                    SystemEnum::from_str(s)
                        .map_err(|_| LockedManifestError::UnrecognizedSystem(s.clone()))
                })
                .collect::<Result<Vec<_>, _>>()?;

            for system in systems {
                // If the package was just added to the manifest, it will be missing in the seed,
                // which is derived from the _previous_ lockfile.
                // In this case, the derivation will be None, and the package will be unconstrained.
                // If the package was already locked, but the descriptor has changed in a way
                // that invalidates the existing resolution, the derivation will be None.
                let locked_derivation = seed_locked_packages
                    .get(&(install_id, &system.to_string()))
                    .filter(|(descriptor, _)| {
                        !descriptor.invalidates_existing_resolution(manifest_descriptor)
                    })
                    .map(|(_, locked_package)| locked_package.derivation.clone());

                let mut resolved_descriptor = resolved_descriptor_base.clone();

                resolved_descriptor.systems = vec![system];
                resolved_descriptor.derivation = locked_derivation;

                resolved_group.descriptors.push(resolved_descriptor);
            }
        }
        Ok(map.into_values())
    }

    /// Eliminate groups that are already fully locked
    /// by extracting them into a separate list of locked packages.
    ///
    /// This is used to avoid re-resolving packages that are already locked.
    fn split_fully_locked_groups(
        groups: impl IntoIterator<Item = PackageGroup>,
        seed_lockfile: Option<&LockedManifestCatalog>,
    ) -> (Vec<LockedPackageCatalog>, Vec<PackageGroup>) {
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        let (already_locked_groups, groups_to_lock): (Vec<_>, Vec<_>) =
            groups.into_iter().partition(|group| {
                group
                    .descriptors
                    .iter()
                    .all(|descriptor| descriptor.derivation.is_some())
            });

        // convert already locked groups back to locked packages
        let already_locked_packages = already_locked_groups
            .iter()
            .flat_map(|group| &group.descriptors)
            .flat_map(|descriptor| {
                std::iter::repeat(&descriptor.install_id).zip(&descriptor.systems)
            })
            .filter_map(|(install_id, system)| {
                seed_locked_packages
                    .get(&(&install_id, &system.to_string()))
                    .map(|(_, locked_package)| (*locked_package).to_owned())
            })
            .collect::<Vec<_>>();

        (already_locked_packages, groups_to_lock)
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
        Ok(groups
            .into_iter()
            .map(|group| {
                if let Some(page) = group.page {
                    if !page.complete {
                        return Err(LockedManifestError::ResolutionFailed(
                            "page wasn't complete".into(),
                        ));
                    }
                    if let Some(pkgs) = page.packages {
                        Ok(pkgs)
                    } else {
                        Err(LockedManifestError::EmptyPage)
                    }
                } else {
                    Err(LockedManifestError::ResolutionFailed(
                        "package group had no page".into(),
                    ))
                }
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .filter_map(|resolved_pkg| {
                let Some(descriptor) = manifest.install.get(&resolved_pkg.install_id).cloned()
                else {
                    debug!(
                        "Package {} is not in the manifest, skipping",
                        resolved_pkg.install_id
                    );
                    return None;
                };

                Some(LockedPackageCatalog::from_parts(resolved_pkg, descriptor))
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
    pub broken: Option<bool>,
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
    #[error("unrecognized system type: {0}")]
    UnrecognizedSystem(String),
    #[error("resolution failed: {0}")]
    ResolutionFailed(String),
    #[error("catalog page was empty")]
    EmptyPage,

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
          systems = ["aarch64-darwin"]
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
            descriptors: vec![PackageDescriptor {
                install_id: "hello_install_id".to_string(),
                attr_path: "hello".to_string(),
                derivation: None,
                version: None,
                allow_pre_releases: None,
                allow_broken: None,
                allow_unfree: None,
                allowed_licenses: None,
                systems: vec![SystemEnum::Aarch64Darwin],
            }],
        }]
    });

    static TEST_RESOLUTION_RESPONSE: Lazy<Vec<ResolvedPackageGroup>> = Lazy::new(|| {
        vec![ResolvedPackageGroup {
            page: Some(CatalogPage {
                complete: true,
                page: 1,
                url: "url".to_string(),
                packages: Some(vec![PackageResolutionInfo {
                    attr_path: "hello".to_string(),
                    broken: Some(false),
                    derivation: "derivation".to_string(),
                    description: Some("description".to_string()),
                    install_id: "hello_install_id".to_string(),
                    license: Some("license".to_string()),
                    locked_url: "locked_url".to_string(),
                    name: "hello".to_string(),
                    outputs: vec![Output {
                        name: "name".to_string(),
                        store_path: "store_path".to_string(),
                    }],
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
                    system: SystemEnum::Aarch64Darwin,
                    stabilities: Some(vec!["stability".to_string()]),
                    unfree: Some(false),
                    version: "version".to_string(),
                }]),
            }),
            name: "group".to_string(),
        }]
    });

    static TEST_LOCKED_MANIFEST: Lazy<LockedManifest> = Lazy::new(|| {
        LockedManifest::Catalog(LockedManifestCatalog {
            version: Version::<1>,
            manifest: TEST_TYPED_MANIFEST.clone(),
            packages: vec![LockedPackageCatalog {
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
                system: SystemEnum::Aarch64Darwin.to_string(),
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
            systems: Some(vec![SystemEnum::Aarch64Darwin.to_string()]),
            version: None,
            priority: None,
            optional: false,
        };

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
            system: SystemEnum::Aarch64Darwin.to_string(),
            group: group.unwrap_or(DEFAULT_GROUP_NAME).to_string(),
            priority: 5,
            optional: false,
        };
        (install_id, descriptor, locked)
    }

    #[test]
    fn make_params_smoke() {
        let manifest = &*TEST_TYPED_MANIFEST;

        let params = LockedManifestCatalog::collect_package_groups(manifest, None)
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
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "emacs".to_string(),
                    derivation: None,
                    install_id: "emacs".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::X8664Linux],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::X8664Linux],
                },
            ],
        }];

        let actual_params = LockedManifestCatalog::collect_package_groups(&manifest, None)
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
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::X8664Linux],
                },
            ],
        }];

        let actual_params = LockedManifestCatalog::collect_package_groups(&manifest, None)
            .unwrap()
            .collect::<Vec<_>>();

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
            emacs.systems = ["aarch64-darwin" ]

            [options]
            systems = ["x86_64-linux"]
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
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "vim".to_string(),
                    derivation: None,
                    install_id: "vim".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::X8664Linux],
                },
            ],
        }];

        let actual_params = LockedManifestCatalog::collect_package_groups(&manifest, None)
            .unwrap()
            .collect::<Vec<_>>();

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
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin],
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
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin],
                }],
            },
        ];

        let actual_params = LockedManifestCatalog::collect_package_groups(&manifest, None)
            .unwrap()
            .collect::<Vec<_>>();

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
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin],
                },
                // The unlocked package should not have a derivation
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "unlocked".to_string(),
                    derivation: None,
                    install_id: "unlocked".to_string(),
                    version: None,
                    allow_broken: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin],
                },
            ],
        }];

        assert_eq!(actual_params, expected_params);
    }

    /// If a seed mapping is provided, use the derivations from the seed where possible
    /// 1) If the package is unchanged, it should not be re-resolved.
    #[test]
    fn make_params_seeded_unchanged() {
        let (foo_before_iid, foo_before_descriptor, foo_before_locked) = fake_package("foo", None);
        let mut manifest_before = manifest::test::empty_catalog_manifest();
        manifest_before
            .install
            .insert(foo_before_iid.clone(), foo_before_descriptor.clone());

        let seed = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest_before.clone(),
            packages: vec![foo_before_locked.clone()],
        };

        // ---------------------------------------------------------------------

        let actual_params =
            LockedManifestCatalog::collect_package_groups(&manifest_before, Some(&seed))
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
        let (foo_before_iid, foo_before_descriptor, foo_before_locked) = fake_package("foo", None);
        let mut manifest_before = manifest::test::empty_catalog_manifest();
        manifest_before
            .install
            .insert(foo_before_iid.clone(), foo_before_descriptor.clone());

        let seed = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest_before.clone(),
            packages: vec![foo_before_locked.clone()],
        };

        // ---------------------------------------------------------------------

        let (foo_after_iid, mut foo_after_descriptor, _) = fake_package("foo", None);
        foo_after_descriptor.pkg_path = "bar".to_string();
        assert!(foo_after_descriptor.invalidates_existing_resolution(&foo_before_descriptor));

        let mut manifest_after = manifest::test::empty_catalog_manifest();
        manifest_after
            .install
            .insert(foo_after_iid.clone(), foo_after_descriptor.clone());

        let actual_params =
            LockedManifestCatalog::collect_package_groups(&manifest_after, Some(&seed))
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
        let (foo_before_iid, foo_before_descriptor, foo_before_locked) = fake_package("foo", None);
        let mut manifest_before = manifest::test::empty_catalog_manifest();
        manifest_before
            .install
            .insert(foo_before_iid.clone(), foo_before_descriptor.clone());

        let seed = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest_before.clone(),
            packages: vec![foo_before_locked.clone()],
        };

        // ---------------------------------------------------------------------

        let (foo_after_iid, mut foo_after_descriptor, _) = fake_package("foo", None);
        foo_after_descriptor.priority = Some(10);
        assert!(!foo_after_descriptor.invalidates_existing_resolution(&foo_before_descriptor));

        let mut manifest_after = manifest::test::empty_catalog_manifest();
        manifest_after
            .install
            .insert(foo_after_iid.clone(), foo_after_descriptor.clone());

        let actual_params =
            LockedManifestCatalog::collect_package_groups(&manifest_after, Some(&seed))
                .unwrap()
                .collect::<Vec<_>>();

        assert_eq!(
            actual_params[0].descriptors[0].derivation.as_ref(),
            Some(&foo_before_locked.derivation)
        );
    }

    #[test]
    fn ungroup_response() {
        let groups = vec![ResolvedPackageGroup {
            page: Some(CatalogPage {
                page: 1,
                complete: true,
                url: "url".to_string(),
                packages: Some(vec![PackageResolutionInfo {
                    attr_path: "hello".to_string(),
                    broken: Some(false),
                    derivation: "derivation".to_string(),
                    description: Some("description".to_string()),
                    install_id: "hello_install_id".to_string(),
                    license: Some("license".to_string()),
                    locked_url: "locked_url".to_string(),
                    name: "hello".to_string(),
                    outputs: vec![Output {
                        name: "name".to_string(),
                        store_path: "store_path".to_string(),
                    }],
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
                    system: SystemEnum::Aarch64Darwin,
                }]),
            }),
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
                groups[0].page.as_ref().unwrap().packages.as_ref().unwrap()[0].clone(),
                manifest
                    .install
                    .get(&groups[0].page.as_ref().unwrap().packages.as_ref().unwrap()[0].install_id)
                    .unwrap()
                    .clone(),
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
        let package_groups: Vec<_> = LockedManifestCatalog::collect_package_groups(&manifest, None)
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
            SystemEnum::Aarch64Darwin,
            SystemEnum::Aarch64Linux,
            SystemEnum::X8664Darwin,
            SystemEnum::X8664Linux,
        ];

        assert_eq!(&*systems, expected_systems.as_slice());
    }

    #[test]
    fn test_split_out_fully_locked_packages() {
        let (foo_iid, foo_descriptor, foo_locked) = fake_package("foo", Some("group1"));
        let (bar_iid, bar_descriptor, bar_locked) = fake_package("bar", Some("group1"));
        let (baz_iid, baz_descriptor, baz_locked) = fake_package("baz", Some("group2"));
        let (yeet_iid, yeet_descriptor, _) = fake_package("yeet", Some("group2"));

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest.install.insert(foo_iid, foo_descriptor.clone());
        manifest.install.insert(bar_iid, bar_descriptor.clone());
        manifest
            .install
            .insert(baz_iid.clone(), baz_descriptor.clone());

        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.clone(), bar_locked.clone(), baz_locked.clone()],
        };

        manifest
            .install
            .insert(yeet_iid.clone(), yeet_descriptor.clone());

        let groups =
            LockedManifestCatalog::collect_package_groups(&manifest, Some(&locked)).unwrap();

        let (fully_locked, to_resolve): (Vec<_>, Vec<_>) =
            LockedManifestCatalog::split_fully_locked_groups(groups, Some(&locked));

        // All packages of group1 are locked
        assert_eq!(&fully_locked, &[bar_locked, foo_locked]);

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
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin,],
                },
                PackageDescriptor {
                    allow_pre_releases: None,
                    attr_path: "yeet".to_string(),
                    derivation: None,
                    install_id: yeet_iid,
                    version: None,
                    allow_broken: None,
                    allow_unfree: None,
                    allowed_licenses: None,
                    systems: vec![SystemEnum::Aarch64Darwin,],
                }
            ],
        }]);
    }

    /// When packages are locked for multiple systems,
    /// locking the same package for fewer systems should drop the extra systems
    #[test]
    fn drop_packages_for_removed_systems() {
        let (foo_iid, foo_descriptor_one_system, foo_locked) = fake_package("foo", Some("group1"));

        assert_eq!(
            foo_descriptor_one_system.systems,
            Some(vec![SystemEnum::Aarch64Darwin.to_string()]),
            "`fake_package` should set the system to [`Aarch64Darwin`]"
        );
        let mut foo_descriptor_two_systems = foo_descriptor_one_system.clone();
        foo_descriptor_two_systems
            .systems
            .as_mut()
            .unwrap()
            .push(SystemEnum::Aarch64Linux.to_string());
        let foo_locked_second_system = LockedPackageCatalog {
            system: SystemEnum::Aarch64Linux.to_string(),
            ..foo_locked.clone()
        };

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest
            .install
            .insert(foo_iid.clone(), foo_descriptor_two_systems.clone());

        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.clone(), foo_locked_second_system.clone()],
        };

        manifest
            .install
            .insert(foo_iid, foo_descriptor_one_system.clone());

        let groups = LockedManifestCatalog::collect_package_groups(&manifest, Some(&locked))
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].descriptors.len(), 1, "Expected only 1 descriptor");
        assert_eq!(
            groups[0].descriptors[0].systems,
            vec![SystemEnum::Aarch64Darwin,],
            "Expected only the Darwin system to be present, second locked system dropped"
        );

        let (fully_locked, to_resolve): (Vec<_>, Vec<_>) =
            LockedManifestCatalog::split_fully_locked_groups(groups, Some(&locked));

        assert_eq!(fully_locked, vec![foo_locked]);
        assert_eq!(to_resolve, vec![]);
    }

    /// Adding another system to a package should invalidate the entire group
    /// such that new systems are resolved with the derivation constraints
    /// of already installed systems
    #[test]
    fn invalidate_group_if_system_added() {
        let (foo_iid, foo_descriptor_one_system, foo_locked) = fake_package("foo", Some("group1"));

        // `fake_package` sets the system to [`Aarch64Darwin`]
        let mut foo_descriptor_two_systems = foo_descriptor_one_system.clone();
        foo_descriptor_two_systems
            .systems
            .as_mut()
            .unwrap()
            .push(SystemEnum::Aarch64Linux.to_string());

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest
            .install
            .insert(foo_iid.clone(), foo_descriptor_one_system.clone());

        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.clone()],
        };

        manifest
            .install
            .insert(foo_iid, foo_descriptor_two_systems.clone());

        let groups = LockedManifestCatalog::collect_package_groups(&manifest, Some(&locked))
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].descriptors.len(),
            2,
            "Expected descriptors for two systems"
        );
        assert_eq!(
            groups[0].descriptors[0].systems,
            vec![SystemEnum::Aarch64Darwin,],
            "Expected only the Darwin system to be present, second locked system dropped"
        );
        assert_eq!(
            groups[0].descriptors[1].systems,
            vec![SystemEnum::Aarch64Linux,],
            "Expected only the Darwin system to be present, second locked system dropped"
        );

        let (fully_locked, to_resolve): (Vec<_>, Vec<_>) =
            LockedManifestCatalog::split_fully_locked_groups(groups, Some(&locked));

        assert_eq!(fully_locked, vec![]);
        assert_eq!(to_resolve.len(), 1);
    }
}
