use catalog_api_v1::types::{MessageLevel, SystemEnum};
use indent::{indent_all_by, indent_by};
use indoc::formatdoc;
use itertools::{Either, Itertools};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;

pub type FlakeRef = Value;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use log::debug;
use thiserror::Error;

use super::environment::UpdateResult;
use super::manifest::{
    Allows,
    ManifestPackageDescriptor,
    ManifestPackageDescriptorCatalog,
    ManifestPackageDescriptorFlake,
    TypedManifestCatalog,
    DEFAULT_GROUP_NAME,
    DEFAULT_PRIORITY,
};
use super::pkgdb::CallPkgDbError;
use crate::data::{CanonicalPath, CanonicalizeError, System, Version};
use crate::flox::Flox;
use crate::models::environment::{global_manifest_lockfile_path, global_manifest_path};
use crate::models::pkgdb::{call_pkgdb, PKGDB_BIN};
use crate::providers::catalog::{
    self,
    CatalogPage,
    MsgAttrPathNotFound,
    PackageDescriptor,
    PackageGroup,
    ResolvedPackageGroup,
};
use crate::providers::flox_cpp_utils::{InstallableLocker, LockedInstallable};
use crate::utils::CommandExt;

static DEFAULT_SYSTEMS_STR: Lazy<[String; 4]> = Lazy::new(|| {
    [
        "aarch64-darwin".to_string(),
        "aarch64-linux".to_string(),
        "x86_64-darwin".to_string(),
        "x86_64-linux".to_string(),
    ]
});

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
    /// locked packages
    pub packages: Vec<LockedPackage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, derive_more::From)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(untagged)]
pub enum LockedPackage {
    Catalog(LockedPackageCatalog),
    Flake(LockedPackageFlake),
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
        }
    }

    pub(crate) fn system(&self) -> &System {
        match self {
            LockedPackage::Catalog(pkg) => &pkg.system,
            LockedPackage::Flake(pkg) => &pkg.locked_installable.locked_system,
        }
    }

    pub fn broken(&self) -> Option<bool> {
        match self {
            LockedPackage::Catalog(pkg) => pkg.broken,
            LockedPackage::Flake(pkg) => pkg.locked_installable.broken,
        }
    }

    pub fn unfree(&self) -> Option<bool> {
        match self {
            LockedPackage::Catalog(pkg) => pkg.unfree,
            LockedPackage::Flake(pkg) => pkg.locked_installable.unfree,
        }
    }
}

#[skip_serializing_none]
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
    // endregion
}

impl LockedPackageCatalog {
    /// Construct a [LockedPackageCatalog] from a [ManifestPackageDescriptor],
    /// the resolved [catalog::PackageResolutionInfo], and corresponding [System].
    ///
    /// There may be more validation/parsing we could do here in the future.
    pub fn from_parts(
        package: catalog::PackageResolutionInfo,
        descriptor: ManifestPackageDescriptorCatalog,
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
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct LockedPackageFlake {
    install_id: String,
    /// Unaltered lock information as returned by `pkgdb lock-flake-installable`.
    /// In this case we completely own the data format in this repo
    /// and so far have to do no conversion.
    /// If this changes in the future, we can add a conversion layer here
    /// similar to [LockedPackageCatalog::from_parts].
    #[serde(flatten)]
    locked_installable: LockedInstallable,
}

impl LockedPackageFlake {
    /// Construct a [LockedPackageFlake] from an [LockedInstallable] and an install_id.
    /// In the future, we may want to pass the original descriptor here as well,
    /// similar to [LockedPackageCatalog::from_parts].
    pub fn from_parts(install_id: String, locked_installable: LockedInstallable) -> Self {
        LockedPackageFlake {
            install_id,
            locked_installable,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct FlakeInstallableToLock {
    install_id: String,
    descriptor: ManifestPackageDescriptorFlake,
    system: System,
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

/// All the resolution failures for a single resolution request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionFailures(pub Vec<ResolutionFailure>);

impl FromIterator<ResolutionFailure> for ResolutionFailures {
    fn from_iter<T: IntoIterator<Item = ResolutionFailure>>(iter: T) -> Self {
        ResolutionFailures(iter.into_iter().collect())
    }
}

/// Data relevant for formatting a resolution failure
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ResolutionFailure {
    PackageNotFound {
        install_id: String,
        attr_path: String,
    },
    PackageUnavailableOnSomeSystems {
        install_id: String,
        attr_path: String,
        invalid_systems: Vec<System>,
        valid_systems: Vec<String>,
    },
    ConstraintsTooTight {
        fallback_msg: String,
        group: String,
    },
    UnknownServiceMessage {
        level: String,
        msg: String,
        context: HashMap<String, String>,
    },
    FallbackMessage {
        msg: String,
    },
}

// Convenience for when you just have a single message
impl From<ResolutionFailure> for ResolutionFailures {
    fn from(value: ResolutionFailure) -> Self {
        ResolutionFailures::from_iter([value])
    }
}

impl Display for ResolutionFailures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let formatted = if self.0.len() > 1 {
            format_multiple_resolution_failures(&self.0)
        } else {
            format_single_resolution_failure(&self.0[0], false)
        };
        write!(f, "{formatted}")
    }
}

/// Formats a single resolution failure in a nice way
fn format_single_resolution_failure(failure: &ResolutionFailure, is_one_of_many: bool) -> String {
    match failure {
        ResolutionFailure::PackageNotFound { attr_path, .. } => {
            // Note: you should never actually see this variant formatted *here* since
            //       it will go through the "didyoumean" mechanism and get formatted there
            format!("could not find package '{attr_path}'.")
        },
        ResolutionFailure::PackageUnavailableOnSomeSystems {
            attr_path,
            invalid_systems,
            valid_systems,
            ..
        } => {
            let extra_indent = if is_one_of_many { 2 } else { 0 };
            let indented_invalid = invalid_systems
                .iter()
                .map(|s| indent_all_by(4, format!("- {s}")))
                .collect::<Vec<_>>()
                .join("\n");
            let indented_valid = valid_systems
                .iter()
                .map(|s| indent_all_by(4, format!("- {s}")))
                .collect::<Vec<_>>()
                .join("\n");
            let listed = [
                format!("package '{attr_path}' not available for"),
                indented_invalid,
                indent_all_by(2, "but it is available for"),
                indented_valid,
            ]
            .join("\n");
            let with_doc_link = formatdoc! {"
            {listed}

            For more on managing system-specific packages, visit the documentation:
            https://flox.dev/docs/tutorials/multi-arch-environments/#handling-unsupported-packages"};
            indent_by(extra_indent, with_doc_link)
        },
        ResolutionFailure::ConstraintsTooTight { group, .. } => {
            let extra_indent = if is_one_of_many { 2 } else { 3 };
            let base_msg = format!("constraints for group '{group}' are too tight");
            let msg = formatdoc! {"
            {}

            Use 'flox edit' to adjust version constraints in the [install] section,
            or isolate dependencies in a new group with '<pkg>.pkg-group = \"newgroup\"'", base_msg};
            indent_by(extra_indent, msg)
        },
        ResolutionFailure::UnknownServiceMessage {
            level: _,
            msg,
            context: _,
        } => {
            if is_one_of_many {
                indent_by(2, msg.to_string())
            } else {
                msg.to_string()
            }
        },
        ResolutionFailure::FallbackMessage { msg } => {
            if is_one_of_many {
                indent_by(2, msg.to_string())
            } else {
                msg.to_string()
            }
        },
    }
}

/// Formats several resolution messages in a more legible way than just one per line
fn format_multiple_resolution_failures(failures: &[ResolutionFailure]) -> String {
    let msgs = failures
        .iter()
        .map(|f| format!("- {}", format_single_resolution_failure(f, true)))
        .collect::<Vec<_>>()
        .join("\n");
    format!("multiple resolution failures:\n{msgs}")
}

impl LockedManifestCatalog {
    /// Convert a locked manifest to a list of installed packages for a given system
    /// in a format shared with the pkgdb based locked manifest.
    pub fn list_packages(&self, system: &System) -> Vec<InstalledPackage> {
        self.packages
            .iter()
            .filter(|package| package.system() == system)
            .cloned()
            .filter_map(|package| match package {
                LockedPackage::Catalog(pkg) => Some(InstalledPackage {
                    install_id: pkg.install_id,
                    rel_path: pkg.attr_path,
                    info: PackageInfo {
                        description: pkg.description,
                        broken: pkg.broken,
                        license: pkg.license,
                        pname: pkg.pname,
                        unfree: pkg.unfree,
                        version: Some(pkg.version),
                    },
                    priority: Some(pkg.priority),
                }),
                LockedPackage::Flake(_) => None,
            })
            .collect()
    }

    /// Produce a lockfile for a given manifest.
    /// Uses the catalog service to resolve [ManifestPackageDescriptorCatalog],
    /// and an [InstallableLocker] to lock [ManifestPackageDescriptorFlake] descriptors.
    ///
    /// If a seed lockfile is provided, packages that are already locked
    /// will constrain the resolution of catalog packages to the same derivation.
    /// Already locked flake installables will not be locked again,
    /// and copied from the seed lockfile as is.
    ///
    /// Catalog and flake installables are locked separately, usinf largely symmetric logic.
    /// Keeping the locking of each kind separate keeps the existing methods simpler
    /// and allows for potential parallelization in the future.
    pub async fn lock_manifest(
        manifest: &TypedManifestCatalog,
        seed_lockfile: Option<&LockedManifestCatalog>,
        client: &impl catalog::ClientTrait,
        installable_locker: &impl InstallableLocker,
    ) -> Result<LockedManifestCatalog, LockedManifestError> {
        let catalog_groups = Self::collect_package_groups(manifest, seed_lockfile)?;
        let (already_locked_packages, groups_to_lock) =
            Self::split_fully_locked_groups(catalog_groups, seed_lockfile);

        let flake_installables = Self::collect_flake_installables(manifest);
        let (already_locked_installables, installables_to_lock) =
            Self::split_locked_flake_installables(flake_installables, seed_lockfile);

        // The manifest could have been edited since locking packages,
        // in which case there may be packages that aren't allowed.
        Self::check_packages_are_allowed(
            already_locked_packages
                .iter()
                .filter_map(LockedPackage::as_catalog_package_ref),
            &manifest.options.allow,
        )?;

        if groups_to_lock.is_empty() && installables_to_lock.is_empty() {
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
        let locked_packages: Vec<LockedPackage> =
            Self::locked_packages_from_resolution(manifest, resolved)?
                .map(Into::into)
                .collect();

        let locked_installables =
            Self::lock_flake_installables(installable_locker, installables_to_lock)?
                .map(Into::into)
                .collect();

        // The server should be checking this,
        // but double check
        Self::check_packages_are_allowed(
            locked_packages
                .iter()
                .filter_map(LockedPackage::as_catalog_package_ref),
            &manifest.options.allow,
        )?;

        let lockfile = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: [
                already_locked_packages,
                locked_packages,
                already_locked_installables,
                locked_installables,
            ]
            .concat(),
        };

        Ok(lockfile)
    }

    /// Given locked packages and manifest options allows, verify that the
    /// locked packages are allowed.
    fn check_packages_are_allowed<'a>(
        locked_packages: impl IntoIterator<Item = &'a LockedPackageCatalog>,
        allow: &Allows,
    ) -> Result<(), LockedManifestError> {
        for package in locked_packages {
            if !allow.licenses.is_empty() {
                let Some(ref license) = package.license else {
                    continue;
                };

                if !allow.licenses.iter().any(|allowed| allowed == license) {
                    return Err(LockedManifestError::LicenseNotAllowed(
                        package.install_id.to_string(),
                        license.to_string(),
                    ));
                }
            }

            // Don't allow broken by default
            if !allow.broken.unwrap_or(false) {
                // Assume a package isn't broken
                if package.broken.unwrap_or(false) {
                    return Err(LockedManifestError::BrokenNotAllowed(
                        package.install_id.to_owned(),
                    ));
                }
            }

            // Allow unfree by default
            if !allow.unfree.unwrap_or(true) {
                // Assume a package isn't unfree
                if package.unfree.unwrap_or(false) {
                    return Err(LockedManifestError::UnfreeNotAllowed(
                        package.install_id.to_owned(),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Transform a lockfile into a mapping that is easier to query:
    /// Lockfile -> { (install_id, system): (package_descriptor, locked_package) }
    fn make_seed_mapping(
        seed: &LockedManifestCatalog,
    ) -> HashMap<(&str, &str), (&ManifestPackageDescriptor, &LockedPackage)> {
        seed.packages
            .iter()
            .filter_map(|locked| {
                let system = locked.system().as_str();
                let install_id = locked.install_id();
                let descriptor = seed.manifest.install.get(locked.install_id())?;
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
    /// only due to parsing [System] strings to [SystemEnum].
    /// If we restricted systems earlier with a common `System` type,
    /// fallible conversions like that would be unnecessary,
    /// or would be pushed higher up.
    fn collect_package_groups(
        manifest: &TypedManifestCatalog,
        seed_lockfile: Option<&LockedManifestCatalog>,
    ) -> Result<impl Iterator<Item = PackageGroup>, LockedManifestError> {
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        // Using a btree map to ensure consistent ordering
        let mut map = BTreeMap::new();

        let manifest_systems = manifest.options.systems.as_deref();

        let maybe_licenses = if manifest.options.allow.licenses.is_empty() {
            None
        } else {
            Some(manifest.options.allow.licenses.clone())
        };

        for (install_id, manifest_descriptor) in manifest.install.iter() {
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

            let systems = {
                let available_systems = manifest_systems.unwrap_or(&*DEFAULT_SYSTEMS_STR);

                let package_systems = manifest_descriptor.systems.as_deref();

                for system in package_systems.into_iter().flatten() {
                    if !available_systems.contains(system) {
                        return Err(LockedManifestError::SystemUnavailableInManifest {
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
                    .map(|s| {
                        SystemEnum::from_str(s)
                            .map_err(|_| LockedManifestError::UnrecognizedSystem(s.to_string()))
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

    /// Eliminate groups that are already fully locked
    /// by extracting them into a separate list of locked packages.
    ///
    /// This is used to avoid re-resolving packages that are already locked.
    fn split_fully_locked_groups(
        groups: impl IntoIterator<Item = PackageGroup>,
        seed_lockfile: Option<&LockedManifestCatalog>,
    ) -> (Vec<LockedPackage>, Vec<PackageGroup>) {
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
        let groups = groups.into_iter().collect::<Vec<_>>();
        let failed_group_indices = Self::detect_failed_resolutions(&groups);
        let failures = if failed_group_indices.is_empty() {
            tracing::debug!("no resolution failures detected");
            None
        } else {
            tracing::debug!("resolution failures detected");
            let failed_groups = failed_group_indices
                .iter()
                .map(|&i| groups[i].clone())
                .collect::<Vec<_>>();
            let failures = Self::collect_failures(&failed_groups, manifest);
            Some(failures)
        };
        if let Some(failures) = failures {
            if !failures.is_empty() {
                tracing::debug!(n = failures.len(), "returning resolution failures");
                return Err(LockedManifestError::ResolutionFailed(ResolutionFailures(
                    failures,
                )));
            }
        }
        let locked_pkg_iter = groups
            .into_iter()
            .flat_map(|group| {
                group
                    .page
                    .and_then(|p| p.packages.clone())
                    .map(|pkgs| pkgs.into_iter())
                    .ok_or(LockedManifestError::ResolutionFailed(
                        // This should be unreachable, otherwise we would have detected
                        // it as a failure
                        ResolutionFailure::FallbackMessage {
                            msg: "catalog page wasn't complete".into(),
                        }
                        .into(),
                    ))
            })
            .flatten()
            .filter_map(|resolved_pkg| {
                manifest
                    .catalog_pkg_descriptor_with_id(&resolved_pkg.install_id)
                    .map(|descriptor| LockedPackageCatalog::from_parts(resolved_pkg, descriptor))
            });
        Ok(locked_pkg_iter)
    }

    /// Constructs [ResolutionFailure]s from the failed groups
    fn collect_failures(
        failed_groups: &[ResolvedPackageGroup],
        manifest: &TypedManifestCatalog,
    ) -> Vec<ResolutionFailure> {
        let mut failures = Vec::new();
        for group in failed_groups {
            tracing::debug!(
                name = group.name,
                "collecting failures from unresolved group"
            );
            for res_msg in group.msgs.iter() {
                match res_msg {
                    catalog::ResolutionMessage::General(inner) => {
                        tracing::debug!(kind = "general", "handling resolution message");
                        // If it's not an error, skip this message
                        if inner.level != MessageLevel::Error {
                            tracing::debug!(
                                level = inner.level.to_string(),
                                msg = inner.msg,
                                "non-error resolution message"
                            );
                            continue;
                        }
                        // If we don't have a level, I guess we have to treat it like an error
                        tracing::debug!("pushing fallback message");
                        let failure = ResolutionFailure::FallbackMessage {
                            msg: inner.msg.clone(),
                        };
                        failures.push(failure);
                    },
                    catalog::ResolutionMessage::AttrPathNotFound(inner) => {
                        tracing::debug!(
                            kind = "attr_path_not_found",
                            "handling resolution message"
                        );
                        if let Some(failure) = Self::attr_path_not_found_failure(inner, manifest) {
                            tracing::debug!("pushing custom failure");
                            failures.push(failure);
                        } else {
                            tracing::debug!("pushing fallback message");
                            let failure = ResolutionFailure::FallbackMessage {
                                msg: inner.msg.clone(),
                            };
                            failures.push(failure);
                        }
                    },
                    catalog::ResolutionMessage::ConstraintsTooTight(inner) => {
                        tracing::debug!(
                            kind = "constraints_too_tight",
                            "handling resolution message"
                        );
                        // If it's not an error, skip this message
                        if inner.level != MessageLevel::Error {
                            tracing::debug!(
                                level = inner.level.to_string(),
                                msg = inner.msg,
                                "non-error resolution message"
                            );
                            continue;
                        }
                        tracing::debug!("pushing fallback message");
                        let failure = ResolutionFailure::ConstraintsTooTight {
                            fallback_msg: inner.msg.clone(),
                            group: group.name.clone(),
                        };
                        failures.push(failure);
                    },
                    catalog::ResolutionMessage::Unknown(inner) => {
                        tracing::debug!(
                            kind = "unknown",
                            level = inner.level.to_string(),
                            msg = inner.msg,
                            msg_type = inner.msg_type,
                            context = serde_json::to_string(&inner.context).unwrap(),
                            "handling unknown resolution message"
                        );
                        // If it's not an error, skip this message
                        if inner.level != MessageLevel::Error {
                            continue;
                        }
                        // If we don't have a level, I guess we have to treat it like an error
                        let failure = ResolutionFailure::UnknownServiceMessage {
                            msg: inner.msg.clone(),
                            level: inner.level.to_string(),
                            context: inner.context.clone(),
                        };
                        failures.push(failure);
                    },
                }
            }
        }
        failures
    }

    /// Converts a "attr path not found" message into different resolution failures
    fn attr_path_not_found_failure(
        r_msg: &MsgAttrPathNotFound,
        manifest: &TypedManifestCatalog,
    ) -> Option<ResolutionFailure> {
        // If we have a level and it's not an error, skip this message
        if r_msg.level != MessageLevel::Error {
            tracing::debug!(
                level = r_msg.level.to_string(),
                msg = r_msg.msg,
                "non-error resolution message"
            );
            return None;
        }
        if r_msg.valid_systems.is_empty() {
            tracing::debug!(
                attr_path = r_msg.attr_path,
                "no valid systems for requested attr_path"
            );
            Some(ResolutionFailure::PackageNotFound {
                install_id: r_msg.install_id.clone(),
                attr_path: r_msg.attr_path.clone(),
            })
        } else {
            let (valid_systems, requested_systems) =
                Self::determine_valid_and_requested_systems(r_msg, manifest)?;
            let mut diff = requested_systems
                .difference(&valid_systems)
                .cloned()
                .collect::<Vec<_>>();
            diff.sort();
            let mut available = r_msg.valid_systems.clone();
            available.sort();
            Some(ResolutionFailure::PackageUnavailableOnSomeSystems {
                install_id: r_msg.install_id.clone(),
                attr_path: r_msg.attr_path.clone(),
                invalid_systems: diff,
                valid_systems: available,
            })
        }
    }

    /// Determines which systems a package is available for and which it was requested on
    fn determine_valid_and_requested_systems(
        r_msg: &MsgAttrPathNotFound,
        manifest: &TypedManifestCatalog,
    ) -> Option<(HashSet<System>, HashSet<System>)> {
        let default_systems = DEFAULT_SYSTEMS_STR
            .clone()
            .into_iter()
            .collect::<HashSet<_>>();
        let valid_systems = r_msg
            .valid_systems
            .clone()
            .into_iter()
            .collect::<HashSet<_>>();
        let manifest_systems = manifest
            .options
            .systems
            .clone()
            .map(|s| s.iter().cloned().collect::<HashSet<_>>())
            .unwrap_or(default_systems);
        let pkg_descriptor = manifest.catalog_pkg_descriptor_with_id(&r_msg.install_id)?;
        let pkg_systems = pkg_descriptor
            .systems
            .map(|s| s.iter().cloned().collect::<HashSet<_>>())
            .clone();
        let requested_systems = if let Some(requested) = pkg_systems {
            requested
        } else {
            manifest_systems
        };
        Some((valid_systems, requested_systems))
    }

    /// Detects whether any groups failed to resolve
    fn detect_failed_resolutions(groups: &[ResolvedPackageGroup]) -> Vec<usize> {
        groups
            .iter()
            .enumerate()
            .filter_map(|(idx, group)| {
                if group.page.is_none() {
                    tracing::debug!(name = group.name, "detected unresolved group");
                    Some(idx)
                } else if group.page.as_ref().is_some_and(|p| !p.complete) {
                    tracing::debug!(name = group.name, "detected incomplete page");
                    Some(idx)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    }

    /// Collect flake installable descriptors from the manifest and create a list of
    /// [FlakeInstallableToLock] to be resolved.
    /// Each descriptor is resolved once per system supported by the manifest,
    /// or other if not specified, for each system in [DEFAULT_SYSTEMS_STR].
    ///
    /// Unlike catalog packages, [FlakeInstallableToLock] are not affected by a seed lockfile.
    /// Already locked flake installables are split from the list in the second step using
    /// [Self::split_locked_flake_installables], based on the descriptor alone,
    /// no additional "marking" is needed.
    fn collect_flake_installables(
        manifest: &TypedManifestCatalog,
    ) -> impl Iterator<Item = FlakeInstallableToLock> + '_ {
        manifest
            .install
            .iter()
            .filter_map(|(install_id, descriptor)| {
                descriptor
                    .as_flake_descriptor_ref()
                    .map(|d| (install_id, d))
            })
            .flat_map(|(iid, d)| {
                let systems = manifest
                    .options
                    .systems
                    .as_deref()
                    .unwrap_or(&*DEFAULT_SYSTEMS_STR);
                systems.iter().map(move |s| FlakeInstallableToLock {
                    install_id: iid.clone(),
                    descriptor: d.clone(),
                    system: s.to_owned(),
                })
            })
    }

    /// Split a list of flake installables into already Locked packages ([LockedPackage])
    /// and yet to lock [FlakeInstallableToLock].
    ///
    /// This is equivalent to [Self::split_fully_locked_groups] but for flake installables.
    /// where `installables` are the flake installables found in a lockfile,
    /// with [Self::collect_flake_installables].
    fn split_locked_flake_installables(
        installables: impl IntoIterator<Item = FlakeInstallableToLock>,
        seed_lockfile: Option<&LockedManifestCatalog>,
    ) -> (Vec<LockedPackage>, Vec<FlakeInstallableToLock>) {
        // todo: consider computing once and passing a reference to the consumer functions.
        //       we now compute this 3 times during a single lock operation
        let seed_locked_packages = seed_lockfile.map_or_else(HashMap::new, Self::make_seed_mapping);

        let by_id = installables.into_iter().group_by(|i| i.install_id.clone());

        let (already_locked, to_lock): (Vec<Vec<LockedPackage>>, Vec<Vec<FlakeInstallableToLock>>) =
            by_id.into_iter().partition_map(|(_, group)| {
                let unlocked = group.collect::<Vec<_>>();
                let mut locked = Vec::new();

                for installable in unlocked.iter() {
                    let Some((locked_descriptor, in_lockfile @ LockedPackage::Flake(_))) =
                        seed_locked_packages
                            .get(&(installable.install_id.as_str(), &installable.system))
                    else {
                        return Either::Right(unlocked);
                    };

                    if ManifestPackageDescriptor::from(installable.descriptor.clone())
                        .invalidates_existing_resolution(locked_descriptor)
                    {
                        return Either::Right(unlocked);
                    }

                    locked.push((*in_lockfile).to_owned());
                }
                Either::Left(locked)
            });

        let already_locked = already_locked.into_iter().flatten().collect();
        let to_lock = to_lock.into_iter().flatten().collect();

        (already_locked, to_lock)
    }

    /// Lock a set of flake installables and return the locked packages.
    /// Errors are collected into [ResolutionFailures] and returned as a single error.
    ///
    /// This is the eequivalent to
    /// [catalog::ClientTrait::resolve] and passing the result to [Self::locked_packages_from_resolution]
    /// in the context of flake installables.
    /// At this point flake installables are resolved sequentially.
    /// In further iterations we may want to resolve them in parallel,
    /// either here, through a method of [InstallableLocker],
    /// or the underlying `pkgdb lock-flake-installable` command itself.
    ///
    /// Todo: [ResolutionFailures] may be caught downstream and used to provide suggestions.
    ///       Those suggestions are invalid for the flake installables case.
    fn lock_flake_installables<'locking>(
        locking: &'locking impl InstallableLocker,
        installables: impl IntoIterator<Item = FlakeInstallableToLock> + 'locking,
    ) -> Result<impl Iterator<Item = LockedPackageFlake> + 'locking, LockedManifestError> {
        let (ok, errs): (Vec<_>, Vec<_>) = installables
            .into_iter()
            .map(|installable| {
                locking
                    .lock_flake_installable(&installable.system, &installable.descriptor.flake)
                    .map(|locked_installable| {
                        LockedPackageFlake::from_parts(installable.install_id, locked_installable)
                    })
            })
            .partition_result();

        if errs.is_empty() {
            Ok(ok.into_iter())
        } else {
            let resolution_failures = errs
                .into_iter()
                .map(|e| ResolutionFailure::FallbackMessage { msg: e.to_string() })
                .collect();

            Err(LockedManifestError::ResolutionFailed(resolution_failures))
        }
    }

    /// Filter out packages from the locked manifest by install_id or group
    ///
    /// This is used to create a seed lockfile to upgrade a subset of packages,
    /// as packages that are not in the seed lockfile will be re-resolved unconstrained.
    pub(crate) fn unlock_packages_by_group_or_iid(&mut self, groups_or_iids: &[&str]) -> &mut Self {
        self.packages = std::mem::take(&mut self.packages)
            .into_iter()
            .filter(|package| {
                if groups_or_iids.contains(&package.install_id()) {
                    return false;
                }

                if let Some(group) = package
                    .as_catalog_package_ref()
                    .map(|pkg| pkg.group.as_str())
                {
                    return !groups_or_iids.contains(&group);
                }

                true
            })
            .collect();

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
#[derive(Debug, Clone, PartialEq)]
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
    #[error("failed to parse check warnings")]
    ParseCheckWarnings(#[source] serde_json::Error),
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

    // todo: this should probably part of some validation logic of the manifest file
    //       rather than occurring during the locking process creation
    #[error("unrecognized system type: {0}")]
    UnrecognizedSystem(String),

    #[error("resolution failed: {0}")]
    ResolutionFailed(ResolutionFailures),
    #[error("catalog page was empty")]
    EmptyPage,

    // todo: this should probably part of some validation logic of the manifest file
    //       rather than occurring during the locking process creation
    #[error(
        "'{install_id}' specifies disabled or unknown system '{system}' (enabled systems: {enabled_systems})",
        enabled_systems=enabled_systems.join(", ")
    )]
    SystemUnavailableInManifest {
        install_id: String,
        system: String,
        enabled_systems: Vec<String>,
    },

    #[error("Catalog lockfile does not support update")]
    UnsupportedLockfileForUpdate,

    #[error("The package '{0}' has license '{1}' which is not in the list of allowed licenses.\n\nAllow this license by adding it to 'options.allow.licenses' in manifest.toml")]
    LicenseNotAllowed(String, String),
    #[error("The package '{0}' is marked as broken.\n\nAllow broken packages by setting 'options.allow.broken = true' in manifest.toml")]
    BrokenNotAllowed(String),
    #[error("The package '{0}' has an unfree license.\n\nAllow unfree packages by setting 'options.allow.unfree = true' in manifest.toml")]
    UnfreeNotAllowed(String),
}

/// A warning produced by `pkgdb manifest check`
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LockfileCheckWarning {
    pub package: String,
    pub message: String,
}

pub mod test_helpers {
    use super::*;

    pub fn fake_catalog_package_lock(
        name: &str,
        group: Option<&str>,
    ) -> (String, ManifestPackageDescriptor, LockedPackageCatalog) {
        let install_id = format!("{}_install_id", name);

        let descriptor = ManifestPackageDescriptorCatalog {
            pkg_path: name.to_string(),
            pkg_group: group.map(|s| s.to_string()),
            systems: Some(vec![SystemEnum::Aarch64Darwin.to_string()]),
            version: None,
            priority: None,
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
            system: SystemEnum::Aarch64Darwin.to_string(),
            group: group.unwrap_or(DEFAULT_GROUP_NAME).to_string(),
            priority: 5,
        };
        (install_id, descriptor, locked)
    }

    pub fn fake_flake_installable_lock(
        name: &str,
    ) -> (String, ManifestPackageDescriptorFlake, LockedPackageFlake) {
        let install_id = format!("{}_install_id", name);

        let descriptor = ManifestPackageDescriptorFlake {
            flake: format!("github:nowhere/exciting#{name}"),
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
                locked_system: "aarch64-darwin".to_string(),
                name: format!("{name}-1.0.0"),
                pname: Some(name.to_string()),
                version: Some("1.0.0".to_string()),
                description: None,
                licenses: None,
                broken: None,
                unfree: None,
            },
        };
        (install_id, descriptor, locked)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::vec;

    use catalog::test_helpers::resolved_pkg_group_with_dummy_package;
    use catalog::{MsgUnknown, ResolutionMessage};
    use catalog_api_v1::types::Output;
    use indoc::indoc;
    use once_cell::sync::Lazy;
    use pretty_assertions::assert_eq;
    use test_helpers::{fake_catalog_package_lock, fake_flake_installable_lock};

    use self::catalog::PackageResolutionInfo;
    use super::*;
    use crate::models::manifest::test::empty_catalog_manifest;
    use crate::models::manifest::{self, RawManifest, TypedManifest};
    use crate::providers::flox_cpp_utils::InstallableLockerMock;

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

    static TEST_RESOLUTION_RESPONSE_UNKNOWN_MSG: Lazy<Vec<ResolvedPackageGroup>> =
        Lazy::new(|| {
            vec![ResolvedPackageGroup {
                page: None,
                name: "group".to_string(),
                msgs: vec![ResolutionMessage::Unknown(MsgUnknown {
                    level: MessageLevel::Error,
                    msg_type: "new_type".to_string(),
                    msg: "User consumable message".to_string(),
                    context: HashMap::new(),
                })],
            }]
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
                msgs: vec![],
            }),
            name: "group".to_string(),
            msgs: vec![],
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
            }
            .into()],
        })
    });

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

        let actual_result = LockedManifestCatalog::collect_package_groups(&manifest, None);

        assert!(
            matches!(actual_result, Err(LockedManifestError::SystemUnavailableInManifest {
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
        manifest.install.insert(
            "unlocked".to_string(),
            ManifestPackageDescriptorCatalog {
                pkg_path: "unlocked".to_string(),
                pkg_group: Some("group".to_string()),
                systems: None,
                version: None,
                priority: None,
            }
            .into(),
        );

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
        let (foo_before_iid, foo_before_descriptor, foo_before_locked) =
            fake_catalog_package_lock("foo", None);
        let mut manifest_before = manifest::test::empty_catalog_manifest();
        manifest_before
            .install
            .insert(foo_before_iid.clone(), foo_before_descriptor.clone());

        let seed = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest_before.clone(),
            packages: vec![foo_before_locked.clone().into()],
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
        let (foo_before_iid, foo_before_descriptor, foo_before_locked) =
            fake_catalog_package_lock("foo", None);
        let mut manifest_before = manifest::test::empty_catalog_manifest();
        manifest_before
            .install
            .insert(foo_before_iid.clone(), foo_before_descriptor.clone());

        let seed = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest_before.clone(),
            packages: vec![foo_before_locked.into()],
        };

        // ---------------------------------------------------------------------

        let (foo_after_iid, mut foo_after_descriptor, _) = fake_catalog_package_lock("foo", None);

        if let ManifestPackageDescriptor::Catalog(ref mut descriptor) = foo_after_descriptor {
            descriptor.pkg_path = "bar".to_string();
        } else {
            panic!("Expected a catalog descriptor");
        };

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
        let (foo_before_iid, foo_before_descriptor, foo_before_locked) =
            fake_catalog_package_lock("foo", None);
        let mut manifest_before = manifest::test::empty_catalog_manifest();
        manifest_before
            .install
            .insert(foo_before_iid.clone(), foo_before_descriptor.clone());

        let seed = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest_before.clone(),
            packages: vec![foo_before_locked.clone().into()],
        };

        // ---------------------------------------------------------------------

        let (foo_after_iid, mut foo_after_descriptor, _) = fake_catalog_package_lock("foo", None);
        if let ManifestPackageDescriptor::Catalog(ref mut descriptor) = foo_after_descriptor {
            descriptor.priority = Some(10);
        } else {
            panic!("Expected a catalog descriptor");
        };

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

    /// If flake installables and catalog packages are mixed,
    /// [LockedManifestCatalog::collect_package_groups]
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
            descriptors: [SystemEnum::Aarch64Darwin, SystemEnum::X8664Linux]
                .map(|system| {
                    [PackageDescriptor {
                        allow_pre_releases: None,
                        attr_path: "vim".to_string(),
                        derivation: None,
                        install_id: "vim".to_string(),
                        version: None,
                        allow_broken: None,
                        allow_unfree: None,
                        allowed_licenses: None,
                        systems: vec![system],
                    }]
                })
                .into_iter()
                .flatten()
                .collect(),
        }];

        let actual_params = LockedManifestCatalog::collect_package_groups(&manifest, None)
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(actual_params, expected_params);
    }

    /// [LockedManifestCatalog::collect_package_groups] generates [FlakeInstallableToLock]
    /// for each default system.
    #[test]
    fn make_installables_to_lock_for_default_systems() {
        let mut manifest = manifest::test::empty_catalog_manifest();
        let (foo_install_id, foo_descriptor, _) = fake_flake_installable_lock("foo");

        manifest
            .install
            .insert(foo_install_id.clone(), foo_descriptor.clone().into());

        let expected = DEFAULT_SYSTEMS_STR
            .clone()
            .map(|system| FlakeInstallableToLock {
                install_id: foo_install_id.clone(),
                descriptor: foo_descriptor.clone(),
                system: system.to_string(),
            });

        let actual: Vec<_> = LockedManifestCatalog::collect_flake_installables(&manifest).collect();

        assert_eq!(actual, expected);
    }

    /// [LockedManifestCatalog::collect_package_groups] generates [FlakeInstallableToLock]
    /// for each system in the manifest.
    #[test]
    fn make_installables_to_lock_for_manifest_systems() {
        let system = "aarch64-darwin";

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest.options.systems = Some(vec![system.to_string()]);

        let (foo_install_id, foo_descriptor, _) = fake_flake_installable_lock("foo");

        manifest
            .install
            .insert(foo_install_id.clone(), foo_descriptor.clone().into());

        let expected = [FlakeInstallableToLock {
            install_id: foo_install_id.clone(),
            descriptor: foo_descriptor.clone(),
            system: system.to_string(),
        }];

        let actual: Vec<_> = LockedManifestCatalog::collect_flake_installables(&manifest).collect();

        assert_eq!(actual, expected);
    }

    /// If flake installables and catalog packages are mixed,
    /// [LockedManifestCatalog::collect_flake_installables]
    /// should only return [FlakeInstallableToLock] for the flake installables.
    #[test]
    fn make_installables_to_lock_filter_catalog() {
        let mut manifest = manifest::test::empty_catalog_manifest();
        let (foo_install_id, foo_descriptor, _) = fake_flake_installable_lock("foo");
        let (bar_install_id, bar_descriptor, _) = fake_catalog_package_lock("bar", None);

        manifest
            .install
            .insert(foo_install_id.clone(), foo_descriptor.clone().into());
        manifest
            .install
            .insert(bar_install_id.clone(), bar_descriptor.clone());

        let expected = DEFAULT_SYSTEMS_STR
            .clone()
            .map(|system| FlakeInstallableToLock {
                install_id: foo_install_id.clone(),
                descriptor: foo_descriptor.clone(),
                system: system.to_string(),
            });

        let actual: Vec<_> = LockedManifestCatalog::collect_flake_installables(&manifest).collect();

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
                msgs: vec![],
            }),
            name: "group".to_string(),
            msgs: vec![],
        }];

        let manifest = &*TEST_TYPED_MANIFEST;

        let locked_packages =
            LockedManifestCatalog::locked_packages_from_resolution(manifest, groups.clone())
                .unwrap()
                .collect::<Vec<_>>();

        let descriptor = manifest
            .install
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

    /// unlocking by iid should remove only the package with that iid
    #[test]
    fn unlock_by_iid() {
        let mut manifest = manifest::test::empty_catalog_manifest();
        let (foo_iid, foo_descriptor, foo_locked) = fake_catalog_package_lock("foo", None);
        let (bar_iid, bar_descriptor, bar_locked) = fake_catalog_package_lock("bar", None);
        manifest.install.insert(foo_iid.clone(), foo_descriptor);
        manifest.install.insert(bar_iid.clone(), bar_descriptor);
        let mut lockfile = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into(), bar_locked.clone().into()],
        };

        lockfile.unlock_packages_by_group_or_iid(&[&foo_iid.clone()]);

        assert_eq!(lockfile.packages, vec![bar_locked.into()]);
    }

    /// Unlocking by group should remove all packages in that group
    #[test]
    fn unlock_by_group() {
        let mut manifest = manifest::test::empty_catalog_manifest();
        let (foo_iid, foo_descriptor, foo_locked) = fake_catalog_package_lock("foo", Some("group"));
        let (bar_iid, bar_descriptor, bar_locked) = fake_catalog_package_lock("bar", Some("group"));
        manifest.install.insert(foo_iid.clone(), foo_descriptor);
        manifest.install.insert(bar_iid.clone(), bar_descriptor);
        let mut lockfile = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into(), bar_locked.into()],
        };

        lockfile.unlock_packages_by_group_or_iid(&["group"]);

        assert_eq!(lockfile.packages, vec![]);
    }

    /// If an unlocked iid is also used as a group, remove both the group
    /// and the package
    #[test]
    fn unlock_by_iid_and_group() {
        let mut manifest = manifest::test::empty_catalog_manifest();
        let (foo_iid, foo_descriptor, foo_locked) =
            fake_catalog_package_lock("foo", Some("foo_install_id"));
        let (bar_iid, bar_descriptor, bar_locked) =
            fake_catalog_package_lock("bar", Some("foo_install_id"));
        manifest.install.insert(foo_iid.clone(), foo_descriptor);
        manifest.install.insert(bar_iid.clone(), bar_descriptor);
        let mut lockfile = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into(), bar_locked.into()],
        };

        lockfile.unlock_packages_by_group_or_iid(&[&foo_iid]);

        assert_eq!(lockfile.packages, vec![]);
    }

    #[test]
    fn unlock_by_iid_noop_if_already_unlocked() {
        let LockedManifest::Catalog(mut seed) = TEST_LOCKED_MANIFEST.clone() else {
            panic!("Expected a catalog lockfile");
        };

        // If the package is not in the seed, the lockfile should be unchanged
        let expected = seed.packages.clone();

        seed.unlock_packages_by_group_or_iid(&["not in here"]);

        assert_eq!(seed.packages, expected,);
    }

    #[tokio::test]
    async fn test_locking_unknown_message() {
        let manifest = &*TEST_TYPED_MANIFEST;

        let mut client = catalog::MockClient::new(None::<String>).unwrap();
        let response = TEST_RESOLUTION_RESPONSE_UNKNOWN_MSG.clone();
        let response_msg: ResolutionMessage =
            response.first().unwrap().msgs.first().unwrap().clone();
        client.push_resolve_response(response);

        let locked_manifest = LockedManifestCatalog::lock_manifest(
            manifest,
            None,
            &client,
            &InstallableLockerMock::new(),
        )
        .await;
        if let Err(LockedManifestError::ResolutionFailed(res_failures)) = locked_manifest {
            if let [ResolutionFailure::UnknownServiceMessage {
                level,
                msg,
                context: _,
            }] = res_failures.0.as_slice()
            {
                assert_eq!(msg, &response_msg.msg());
                assert_eq!(level, "error");
            } else {
                panic!();
            }
        } else {
            panic!();
        }
    }

    #[tokio::test]
    async fn test_locking_1() {
        let manifest = &*TEST_TYPED_MANIFEST;

        let mut client = catalog::MockClient::new(None::<String>).unwrap();
        client.push_resolve_response(TEST_RESOLUTION_RESPONSE.clone());

        let locked_manifest = LockedManifestCatalog::lock_manifest(
            manifest,
            None,
            &client,
            &InstallableLockerMock::new(),
        )
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
        let (foo_iid, foo_descriptor, foo_locked) =
            fake_catalog_package_lock("foo", Some("group1"));
        let (bar_iid, bar_descriptor, bar_locked) =
            fake_catalog_package_lock("bar", Some("group1"));
        let (baz_iid, baz_descriptor, baz_locked) =
            fake_catalog_package_lock("baz", Some("group2"));
        let (yeet_iid, yeet_descriptor, _) = fake_catalog_package_lock("yeet", Some("group2"));

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest.install.insert(foo_iid, foo_descriptor.clone());
        manifest.install.insert(bar_iid, bar_descriptor.clone());
        manifest
            .install
            .insert(baz_iid.clone(), baz_descriptor.clone());

        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: [&foo_locked, &bar_locked, &baz_locked]
                .map(|p| p.clone().into())
                .to_vec(),
        };

        manifest
            .install
            .insert(yeet_iid.clone(), yeet_descriptor.clone());

        let groups =
            LockedManifestCatalog::collect_package_groups(&manifest, Some(&locked)).unwrap();

        let (fully_locked, to_resolve): (Vec<_>, Vec<_>) =
            LockedManifestCatalog::split_fully_locked_groups(groups, Some(&locked));

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
        let (foo_iid, foo_descriptor_one_system, foo_locked) =
            fake_catalog_package_lock("foo", Some("group1"));

        let systems = &foo_descriptor_one_system
            .as_catalog_descriptor_ref()
            .expect("expected a catalog descriptor")
            .systems;

        assert_eq!(
            systems,
            &Some(vec![SystemEnum::Aarch64Darwin.to_string()]),
            "`fake_package` should set the system to [`Aarch64Darwin`]"
        );

        let mut foo_descriptor_two_systems = foo_descriptor_one_system.clone();

        if let ManifestPackageDescriptor::Catalog(descriptor) = &mut foo_descriptor_two_systems {
            descriptor
                .systems
                .as_mut()
                .unwrap()
                .push(SystemEnum::Aarch64Linux.to_string());
        } else {
            panic!("Expected a catalog descriptor");
        };

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
            packages: vec![
                foo_locked.clone().into(),
                foo_locked_second_system.clone().into(),
            ],
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
            "Expected only the Darwin system to be present"
        );

        let (fully_locked, to_resolve): (Vec<_>, Vec<_>) =
            LockedManifestCatalog::split_fully_locked_groups(groups, Some(&locked));

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
                .push(SystemEnum::Aarch64Linux.to_string());
        } else {
            panic!("Expected a catalog descriptor");
        };

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest
            .install
            .insert(foo_iid.clone(), foo_descriptor_one_system.clone());

        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into()],
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
        assert_eq!(groups[0].descriptors[0].systems, vec![
            SystemEnum::Aarch64Darwin
        ]);
        assert_eq!(groups[0].descriptors[1].systems, vec![
            SystemEnum::Aarch64Linux
        ]);

        let (fully_locked, to_resolve): (Vec<_>, Vec<_>) =
            LockedManifestCatalog::split_fully_locked_groups(groups, Some(&locked));

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

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest.options.systems = Some(vec![system.to_string()]);

        manifest
            .install
            .insert(foo_iid.clone(), foo_descriptor.clone().into());
        manifest
            .install
            .insert(bar_iid.clone(), bar_descriptor.clone().into());

        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![bar_locked.clone().into()],
        };

        let flake_installables = LockedManifestCatalog::collect_flake_installables(&manifest);

        let (locked, to_resolve): (Vec<_>, Vec<_>) =
            LockedManifestCatalog::split_locked_flake_installables(
                flake_installables,
                Some(&locked),
            );

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

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest.options.systems = Some(vec![system.to_string()]);

        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![bar_locked.clone().into()],
        };

        let flake_installables = LockedManifestCatalog::collect_flake_installables(&manifest);

        let (locked, to_resolve): (Vec<_>, Vec<_>) =
            LockedManifestCatalog::split_locked_flake_installables(
                flake_installables,
                Some(&locked),
            );

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
        foo_locked_system_2.locked_installable.locked_system = SystemEnum::Aarch64Linux.to_string();

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest.options.systems = Some(vec![system.to_string()]);

        manifest
            .install
            .insert(foo_iid.clone(), foo_descriptor.clone().into());

        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![
                foo_locked_system_1.clone().into(),
                foo_locked_system_2.into(),
            ],
        };

        let flake_installables = LockedManifestCatalog::collect_flake_installables(&manifest);

        let (locked, to_resolve): (Vec<_>, Vec<_>) =
            LockedManifestCatalog::split_locked_flake_installables(
                flake_installables,
                Some(&locked),
            );

        assert_eq!(locked, vec![foo_locked_system_1.into()]);
        assert_eq!(&to_resolve, &[]);
    }

    /// If a system is added to the manifest, the package should be reresolved for all systems
    #[test]
    fn invalidate_locked_flake_if_system_added() {
        let system_1 = "aarch64-darwin";
        let system_2 = "aarch64-linux";
        let (foo_iid, foo_descriptor, foo_locked) = fake_flake_installable_lock("foo");

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest
            .install
            .insert(foo_iid.clone(), foo_descriptor.clone().into());

        // lockfile for only system_1
        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.clone().into()],
        };

        // system_2 is added to the manifest
        manifest.options.systems = Some(vec![system_1.to_string(), system_2.to_string()]);

        let flake_installables = LockedManifestCatalog::collect_flake_installables(&manifest);

        let (locked, to_resolve): (Vec<_>, Vec<_>) =
            LockedManifestCatalog::split_locked_flake_installables(
                flake_installables,
                Some(&locked),
            );

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

    /// [LockedManifestCatalog::lock_manifest] returns an error if an already
    /// locked package is no longer allowed
    #[tokio::test]
    async fn lock_manifest_catches_not_allowed_package() {
        // Create a manifest and lockfile with an unfree package foo.
        // Don't set `options.allow.unfree`
        let (foo_iid, foo_descriptor_one_system, mut foo_locked) =
            fake_catalog_package_lock("foo", None);
        foo_locked.unfree = Some(true);
        let mut manifest = empty_catalog_manifest();
        manifest
            .install
            .insert(foo_iid.clone(), foo_descriptor_one_system.clone());

        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest: manifest.clone(),
            packages: vec![foo_locked.into()],
        };

        // Set `options.allow.unfree = false` in the manifest, but not the lockfile
        manifest.options.allow.unfree = Some(false);

        let client = catalog::MockClient::new(None::<String>).unwrap();
        assert!(matches!(
            LockedManifestCatalog::lock_manifest(
                &manifest,
                Some(&locked),
                &client,
                &InstallableLockerMock::new()
            )
            .await
            .unwrap_err(),
            LockedManifestError::UnfreeNotAllowed { .. }
        ));
    }

    /// [LockedManifestCatalog::lock_manifest] returns an error if the server
    /// returns a package that is not allowed.
    #[tokio::test]
    async fn lock_manifest_catches_not_allowed_package_from_server() {
        // Create a manifest with a package foo and `options.allow.unfree = false`
        let (foo_iid, foo_descriptor_one_system, _) =
            fake_catalog_package_lock("foo", Some("toplevel"));
        let mut manifest = empty_catalog_manifest();
        manifest
            .install
            .insert(foo_iid.clone(), foo_descriptor_one_system.clone());
        manifest.options.allow.unfree = Some(false);

        // Return a response that says foo is unfree. If this happens, it's a bug in the server
        let mut client = catalog::MockClient::new(None::<String>).unwrap();
        let mut resolved_group = resolved_pkg_group_with_dummy_package(
            "toplevel",
            // TODO: this is hardcoded in fake_package
            &System::from("aarch64-darwin"),
            &foo_iid,
            "foo",
            "0",
        );
        resolved_group
            .page
            .as_mut()
            .unwrap()
            .packages
            .as_mut()
            .unwrap()[0]
            .unfree = Some(true);
        client.push_resolve_response(vec![resolved_group]);
        assert!(matches!(
            LockedManifestCatalog::lock_manifest(
                &manifest,
                None,
                &client,
                &InstallableLockerMock::new()
            )
            .await
            .unwrap_err(),
            LockedManifestError::UnfreeNotAllowed { .. }
        ));
    }

    /// [LockedManifestCatalog::check_packages_are_allowed] returns an error
    /// when it finds a disallowed license
    #[test]
    fn check_packages_are_allowed_disallowed_license() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.license = Some("disallowed".to_string());

        assert!(matches!(
            LockedManifestCatalog::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: None,
                licenses: vec!["allowed".to_string()]
            }),
            Err(LockedManifestError::LicenseNotAllowed { .. })
        ));
    }

    /// [LockedManifestCatalog::check_packages_are_allowed] does not error when
    /// a package's license is allowed
    #[test]
    fn check_packages_are_allowed_allowed_license() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.license = Some("allowed".to_string());

        assert!(
            LockedManifestCatalog::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: None,
                licenses: vec!["allowed".to_string()]
            })
            .is_ok()
        );
    }

    /// [LockedManifestCatalog::check_packages_are_allowed] returns an error
    /// when a package is broken even if `allow.broken` is unset
    #[test]
    fn check_packages_are_allowed_broken_default() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.broken = Some(true);

        assert!(matches!(
            LockedManifestCatalog::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: None,
                licenses: vec![]
            }),
            Err(LockedManifestError::BrokenNotAllowed { .. })
        ));
    }

    /// [LockedManifestCatalog::check_packages_are_allowed] does not error for a
    /// broken package when `allow.broken = true`
    #[test]
    fn check_packages_are_allowed_broken_true() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.broken = Some(true);

        assert!(
            LockedManifestCatalog::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: Some(true),
                licenses: vec![]
            })
            .is_ok()
        );
    }

    /// [LockedManifestCatalog::check_packages_are_allowed] returns an error
    /// when a package is broken and `allow.broken = false`
    #[test]
    fn check_packages_are_allowed_broken_false() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.broken = Some(true);

        assert!(matches!(
            LockedManifestCatalog::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: Some(false),
                licenses: vec![]
            }),
            Err(LockedManifestError::BrokenNotAllowed { .. })
        ));
    }

    /// [LockedManifestCatalog::check_packages_are_allowed] does not error for
    /// an unfree package when `allow.unfree` is unset
    #[test]
    fn check_packages_are_allowed_unfree_default() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.unfree = Some(true);

        assert!(
            LockedManifestCatalog::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: None,
                broken: None,
                licenses: vec![]
            })
            .is_ok()
        );
    }

    /// [LockedManifestCatalog::check_packages_are_allowed] does not error for a
    /// an unfree package when `allow.unfree = true`
    #[test]
    fn check_packages_are_allowed_unfree_true() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.unfree = Some(true);

        assert!(
            LockedManifestCatalog::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: Some(true),
                broken: None,
                licenses: vec![]
            })
            .is_ok()
        );
    }

    /// [LockedManifestCatalog::check_packages_are_allowed] returns an error
    /// when a package is unfree and `allow.unfree = false`
    #[test]
    fn check_packages_are_allowed_unfree_false() {
        let (_, _, mut foo_locked) = fake_catalog_package_lock("foo", None);
        foo_locked.unfree = Some(true);

        assert!(matches!(
            LockedManifestCatalog::check_packages_are_allowed(&vec![foo_locked], &Allows {
                unfree: Some(false),
                broken: None,
                licenses: vec![]
            }),
            Err(LockedManifestError::UnfreeNotAllowed { .. })
        ));
    }

    #[test]
    fn test_list_packages() {
        let (foo_iid, foo_descriptor, foo_locked) =
            fake_catalog_package_lock("foo", Some("group1"));
        let (bar_iid, bar_descriptor, bar_locked) =
            fake_catalog_package_lock("bar", Some("group1"));
        let (baz_iid, mut baz_descriptor, mut baz_locked) =
            fake_catalog_package_lock("baz", Some("group2"));

        if let ManifestPackageDescriptor::Catalog(ref mut descriptor) = baz_descriptor {
            descriptor.systems = Some(vec![SystemEnum::Aarch64Linux.to_string()]);
        } else {
            panic!("Expected a catalog descriptor");
        };
        baz_locked.system = SystemEnum::Aarch64Linux.to_string();

        let mut manifest = manifest::test::empty_catalog_manifest();
        manifest
            .install
            .insert(foo_iid.clone(), foo_descriptor.clone());
        manifest
            .install
            .insert(bar_iid.clone(), bar_descriptor.clone());
        manifest
            .install
            .insert(baz_iid.clone(), baz_descriptor.clone());

        let locked = LockedManifestCatalog {
            version: Version::<1>,
            manifest,
            packages: vec![
                foo_locked.clone().into(),
                bar_locked.clone().into(),
                baz_locked.clone().into(),
            ],
        };

        let foo_pkg_path = foo_descriptor
            .unwrap_catalog_descriptor()
            .expect("expected catalog descriptor")
            .pkg_path;

        let bar_pkg_path = bar_descriptor
            .unwrap_catalog_descriptor()
            .expect("expected a catalog descriptor")
            .pkg_path;

        let actual = locked.list_packages(&SystemEnum::Aarch64Darwin.to_string());
        let expected = [
            InstalledPackage {
                install_id: foo_iid,
                rel_path: foo_pkg_path,
                info: PackageInfo {
                    description: foo_locked.description,
                    broken: foo_locked.broken,
                    license: foo_locked.license,
                    pname: foo_locked.pname,
                    unfree: foo_locked.unfree,
                    version: Some(foo_locked.version),
                },
                priority: Some(foo_locked.priority),
            },
            InstalledPackage {
                install_id: bar_iid,
                rel_path: bar_pkg_path,
                info: PackageInfo {
                    description: bar_locked.description,
                    broken: bar_locked.broken,
                    license: bar_locked.license,
                    pname: bar_locked.pname,
                    unfree: bar_locked.unfree,
                    version: Some(bar_locked.version),
                },
                priority: Some(bar_locked.priority),
            },
            // baz is not in the list because it is not available for the requested system
        ];

        assert_eq!(&actual, &expected);
    }
}
