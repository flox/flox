use std::collections::{BTreeMap, BTreeSet};

use flox_core::Version;
use indoc::formatdoc;
use itertools::Itertools;
#[cfg(test)]
use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::data::System;
use crate::providers::services::ServiceError;
#[cfg(test)]
use crate::utils::proptest_btree_map_alphanum_keys;

pub(crate) const DEFAULT_GROUP_NAME: &str = "toplevel";
pub const DEFAULT_PRIORITY: u64 = 5;

pub trait Inner {
    type Inner;

    fn inner(&self) -> &Self::Inner;
    fn inner_mut(&mut self) -> &mut Self::Inner;
    fn into_inner(self) -> Self::Inner;
}

macro_rules! impl_into_inner {
    ($wrapper:ty, $inner_type:ty) => {
        impl Inner for $wrapper {
            type Inner = $inner_type;

            fn inner(&self) -> &Self::Inner {
                &self.0
            }

            fn inner_mut(&mut self) -> &mut Self::Inner {
                &mut self.0
            }

            fn into_inner(self) -> Self::Inner {
                self.0
            }
        }
    };
}

/// Not meant for writing manifest files, only for reading them.
/// Modifications should be made using the the raw functions in this module.

// We use skip_serializing_if throughout to reduce the size of the lockfile and
// improve backwards compatibility when we introduce fields.
// We don't use Option and skip_serializing_none because an empty table gets
// treated as Some,
// but we don't care about distinguishing between a table not being present and
// a table being present but empty.
// In both cases, we can just skip serializing.
// It would be better if we could deny_unknown_fields when we're deserializing
// the user provided manifest but allow unknown fields when deserializing the
// lockfile,
// but that doesn't seem worth the effort at the moment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: Version<1>,
    /// The packages to install in the form of a map from install_id
    /// to package descriptor.
    #[serde(default)]
    #[serde(skip_serializing_if = "ManifestInstall::skip_serializing")]
    pub install: ManifestInstall,
    /// Variables that are exported to the shell environment upon activation.
    #[serde(default)]
    #[serde(skip_serializing_if = "ManifestVariables::skip_serializing")]
    pub vars: ManifestVariables,
    /// Hooks that are run at various times during the lifecycle of the manifest
    /// in a known shell environment.
    #[serde(default)]
    pub hook: ManifestHook,
    /// Profile scripts that are run in the user's shell upon activation.
    #[serde(default)]
    pub profile: ManifestProfile,
    /// Options that control the behavior of the manifest.
    #[serde(default)]
    pub options: ManifestOptions,
    /// Service definitions
    #[serde(default)]
    #[serde(skip_serializing_if = "ManifestServices::skip_serializing")]
    pub services: ManifestServices,
    /// Package build definitions
    #[serde(default)]
    #[serde(skip_serializing_if = "ManifestBuild::skip_serializing")]
    pub build: ManifestBuild,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub containerize: Option<ManifestContainerize>,
}

impl Manifest {
    /// Get the package descriptor with the specified install_id.
    pub fn pkg_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<ManifestPackageDescriptor> {
        self.install.0.get(id.as_ref()).cloned()
    }

    /// Get the package descriptor with the specified install_id.
    pub fn catalog_pkg_descriptor_with_id(
        &self,
        id: impl AsRef<str>,
    ) -> Option<ManifestPackageDescriptorCatalog> {
        self.install
            .0
            .get(id.as_ref())
            .and_then(ManifestPackageDescriptor::as_catalog_descriptor_ref)
            .cloned()
    }

    /// Get the package descriptor with the specified install_id.
    pub fn flake_pkg_descriptor_with_id(
        &self,
        id: impl AsRef<str>,
    ) -> Option<ManifestPackageDescriptor> {
        self.install.0.get(id.as_ref()).cloned()
    }

    /// Get the package descriptors in the "toplevel" group.
    pub fn pkg_descriptors_in_toplevel_group(&self) -> Vec<(String, ManifestPackageDescriptor)> {
        pkg_descriptors_in_toplevel_group(&self.install.0)
    }

    /// Get the package descriptors in a named group.
    pub fn pkg_descriptors_in_named_group(
        &self,
        name: impl AsRef<str>,
    ) -> Vec<(String, ManifestPackageDescriptor)> {
        pkg_descriptors_in_named_group(name, &self.install.0)
    }

    /// Check whether the specified name is either an install_id or group name.
    pub fn pkg_or_group_found_in_manifest(&self, name: impl AsRef<str>) -> bool {
        pkg_or_group_found_in_manifest(name.as_ref(), &self.install.0)
    }

    /// Check whether the specified package belongs to a named group
    /// with additional packages.
    pub fn pkg_belongs_to_non_empty_named_group(
        &self,
        pkg: impl AsRef<str>,
    ) -> Result<Option<String>, ManifestError> {
        pkg_belongs_to_non_empty_named_group(pkg.as_ref(), &self.install.0)
    }

    /// Check whether the specified package belongs to the "toplevel" group
    /// with additional packages.
    pub fn pkg_belongs_to_non_empty_toplevel_group(
        &self,
        pkg: impl AsRef<str>,
    ) -> Result<bool, ManifestError> {
        pkg_belongs_to_non_empty_toplevel_group(pkg.as_ref(), &self.install.0)
    }
}

pub(crate) fn pkg_descriptors_in_toplevel_group(
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> Vec<(String, ManifestPackageDescriptor)> {
    descriptors
        .iter()
        .filter(|(_, desc)| {
            let ManifestPackageDescriptor::Catalog(ManifestPackageDescriptorCatalog {
                pkg_group,
                ..
            }) = desc
            else {
                return false;
            };

            pkg_group.is_none()
        })
        .map(|(id, desc)| (id.clone(), desc.clone()))
        .collect::<Vec<_>>()
}

pub(crate) fn pkg_descriptors_in_named_group(
    name: impl AsRef<str>,
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> Vec<(String, ManifestPackageDescriptor)> {
    descriptors
        .iter()
        .filter(|(_, desc)| {
            let ManifestPackageDescriptor::Catalog(ManifestPackageDescriptorCatalog {
                pkg_group,
                ..
            }) = desc
            else {
                return false;
            };

            pkg_group
                .as_ref()
                .is_some_and(|n| n.as_str() == name.as_ref())
        })
        .map(|(id, desc)| (id.clone(), desc.clone()))
        .collect::<Vec<_>>()
}

/// Scans the provided package descriptors to determine if the search term is a package or
/// group in the manifest.
fn pkg_or_group_found_in_manifest(
    search_term: impl AsRef<str>,
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> bool {
    descriptors.iter().any(|(id, desc)| {
        let group = if let ManifestPackageDescriptor::Catalog(catalog) = desc {
            catalog.pkg_group.as_deref()
        } else {
            None
        };

        let search_term = search_term.as_ref();

        (search_term == id.as_str()) || (Some(search_term) == group)
    })
}

/// named group in the manifest with other packages.
fn pkg_belongs_to_non_empty_named_group(
    pkg: &str,
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> Result<Option<String>, ManifestError> {
    let descriptor = descriptors
        .get(pkg)
        .ok_or(ManifestError::PkgOrGroupNotFound(pkg.to_string()))?;

    let ManifestPackageDescriptor::Catalog(ManifestPackageDescriptorCatalog { pkg_group, .. }) =
        descriptor
    else {
        return Ok(None);
    };

    let Some(ref group) = pkg_group else {
        return Ok(None);
    };
    let pkgs = pkg_descriptors_in_named_group(group, descriptors);
    let other_pkgs_in_group = pkgs.iter().any(|(id, _)| id != pkg);
    if other_pkgs_in_group {
        Ok(Some(group.clone()))
    } else {
        Ok(None)
    }
}

/// Scans the provided package descriptors to determine if the specified package belongs to
/// the "toplevel" group with other packages.
fn pkg_belongs_to_non_empty_toplevel_group(
    pkg: &str,
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> Result<bool, ManifestError> {
    descriptors
        .get(pkg)
        .ok_or(ManifestError::PkgOrGroupNotFound(pkg.to_string()))?;
    let pkgs = pkg_descriptors_in_toplevel_group(descriptors);
    let other_toplevel_packages_exist = pkgs.iter().any(|(id, _)| id != pkg);
    Ok(other_toplevel_packages_exist)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestInstall(
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest_btree_map_alphanum_keys::<ManifestPackageDescriptor>(10, 3)"
        )
    )]
    pub(crate) BTreeMap<String, ManifestPackageDescriptor>,
);

impl ManifestInstall {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

impl_into_inner!(ManifestInstall, BTreeMap<String, ManifestPackageDescriptor>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
// todo: this can make the error messages less clear and might call for a custom (de)serialize impl
#[serde(
    untagged,
    expecting = "Expected either a catalog package descriptor, a flake installable or a store path.
See https://flox.dev/docs/concepts/manifest/#package-descriptors for more information."
)]
pub enum ManifestPackageDescriptor {
    Catalog(ManifestPackageDescriptorCatalog),
    FlakeRef(ManifestPackageDescriptorFlake),
    StorePath(ManifestPackageDescriptorStorePath),
}

impl ManifestPackageDescriptor {
    /// Check if two package descriptors should have the same resolution.
    /// This is used to determine if a package needs to be re-resolved
    /// in the presence of an existing lock.
    ///
    /// * Descriptors are resolved per system,
    ///   changing the supported systems does not invalidate _existing_ resolutions.
    /// * Priority is not used in resolution, so it is ignored.
    pub(crate) fn invalidates_existing_resolution(&self, other: &Self) -> bool {
        use ManifestPackageDescriptor::*;
        match (self, other) {
            (Catalog(this), Catalog(other)) => this.invalidates_existing_resolution(other),
            (FlakeRef(this), FlakeRef(other)) => this != other,
            // different types of descriptors are always different
            _ => true,
        }
    }

    #[must_use]
    pub fn unwrap_catalog_descriptor(self) -> Option<ManifestPackageDescriptorCatalog> {
        match self {
            ManifestPackageDescriptor::Catalog(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_catalog_descriptor_ref(&self) -> Option<&ManifestPackageDescriptorCatalog> {
        match self {
            ManifestPackageDescriptor::Catalog(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn unwrap_flake_descriptor(self) -> Option<ManifestPackageDescriptorFlake> {
        match self {
            ManifestPackageDescriptor::FlakeRef(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_flake_descriptor_ref(&self) -> Option<&ManifestPackageDescriptorFlake> {
        match self {
            ManifestPackageDescriptor::FlakeRef(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn unwrap_store_path_descriptor(self) -> Option<ManifestPackageDescriptorStorePath> {
        match self {
            ManifestPackageDescriptor::StorePath(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_store_path_descriptor_ref(&self) -> Option<&ManifestPackageDescriptorStorePath> {
        match self {
            ManifestPackageDescriptor::StorePath(descriptor) => Some(descriptor),
            _ => None,
        }
    }
}

impl From<&ManifestPackageDescriptorCatalog> for ManifestPackageDescriptor {
    fn from(val: &ManifestPackageDescriptorCatalog) -> Self {
        ManifestPackageDescriptor::Catalog(val.clone())
    }
}

impl From<ManifestPackageDescriptorCatalog> for ManifestPackageDescriptor {
    fn from(val: ManifestPackageDescriptorCatalog) -> Self {
        ManifestPackageDescriptor::Catalog(val)
    }
}

impl From<&ManifestPackageDescriptorFlake> for ManifestPackageDescriptor {
    fn from(val: &ManifestPackageDescriptorFlake) -> Self {
        ManifestPackageDescriptor::FlakeRef(val.clone())
    }
}

impl From<ManifestPackageDescriptorFlake> for ManifestPackageDescriptor {
    fn from(val: ManifestPackageDescriptorFlake) -> Self {
        ManifestPackageDescriptor::FlakeRef(val)
    }
}

impl From<&ManifestPackageDescriptorStorePath> for ManifestPackageDescriptor {
    fn from(val: &ManifestPackageDescriptorStorePath) -> Self {
        ManifestPackageDescriptor::StorePath(val.clone())
    }
}

impl From<ManifestPackageDescriptorStorePath> for ManifestPackageDescriptor {
    fn from(val: ManifestPackageDescriptorStorePath) -> Self {
        ManifestPackageDescriptor::StorePath(val)
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestPackageDescriptorCatalog {
    pub(crate) pkg_path: String,
    pub(crate) pkg_group: Option<String>,
    #[cfg_attr(test, proptest(strategy = "proptest::option::of(0..10u64)"))]
    pub(crate) priority: Option<u64>,
    pub(crate) version: Option<String>,
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::option::of(proptest::collection::vec(any::<System>(), 1..3))"
        )
    )]
    pub(crate) systems: Option<Vec<System>>,
}

impl ManifestPackageDescriptorCatalog {
    /// Check if two package descriptors should have the same resolution.
    /// This is used to determine if a package needs to be re-resolved
    /// in the presence of an existing lock.
    ///
    /// * Descriptors are resolved per system,
    ///   changing the supported systems does not invalidate _existing_ resolutions.
    /// * Priority is not used in resolution, so it is ignored.
    pub(super) fn invalidates_existing_resolution(&self, other: &Self) -> bool {
        // unpack to avoid forgetting to update this method when new fields are added
        let ManifestPackageDescriptorCatalog {
            pkg_path,
            pkg_group,
            version,
            systems: _,
            priority: _,
        } = self;

        pkg_path != &other.pkg_path || pkg_group != &other.pkg_group || version != &other.version
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestPackageDescriptorFlake {
    pub flake: String,
    #[cfg_attr(test, proptest(strategy = "proptest::option::of(0..10u64)"))]
    pub(crate) priority: Option<u64>,
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::option::of(proptest::collection::vec(any::<System>(), 1..3))"
        )
    )]
    pub(crate) systems: Option<Vec<System>>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestPackageDescriptorStorePath {
    pub(crate) store_path: String,
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::option::of(proptest::collection::vec(any::<System>(), 1..3))"
        )
    )]
    pub(crate) systems: Option<Vec<System>>,
    #[cfg_attr(test, proptest(strategy = "proptest::option::of(0..10u64)"))]
    pub(crate) priority: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestVariables(
    #[cfg_attr(
        test,
        proptest(strategy = "proptest_btree_map_alphanum_keys::<String>(10, 3)")
    )]
    pub(crate) BTreeMap<String, String>,
);

impl ManifestVariables {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

impl_into_inner!(ManifestVariables, BTreeMap<String, String>);

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestHook {
    /// A script that is run at activation time,
    /// in a flox provided bash shell
    pub(crate) on_activate: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct ManifestProfile {
    /// When defined, this hook is run by _all_ shells upon activation
    pub(crate) common: Option<String>,
    /// When defined, this hook is run upon activation in a bash shell
    pub(crate) bash: Option<String>,
    /// When defined, this hook is run upon activation in a zsh shell
    pub(crate) zsh: Option<String>,
    /// When defined, this hook is run upon activation in a fish shell
    pub(crate) fish: Option<String>,
    /// When defined, this hook is run upon activation in a tcsh shell
    pub(crate) tcsh: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestOptions {
    /// A list of systems that each package is resolved for.
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::option::of(proptest::collection::vec(any::<System>(), 1..4))"
        )
    )]
    pub systems: Option<Vec<System>>,
    /// Options that control what types of packages are allowed.
    #[serde(default)]
    pub allow: Allows,
    /// Options that control how semver versions are resolved.
    #[serde(default)]
    pub semver: SemverOptions,
    pub cuda_detection: Option<bool>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct Allows {
    /// Whether to allow packages that are marked as `unfree`
    pub unfree: Option<bool>,
    /// Whether to allow packages that are marked as `broken`
    pub broken: Option<bool>,
    /// A list of license descriptors that are allowed
    #[serde(default)]
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::collection::vec(any::<String>(), 0..3)")
    )]
    pub licenses: Vec<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct SemverOptions {
    /// Whether to allow pre-release versions when resolving
    #[serde(default)]
    pub allow_pre_releases: Option<bool>,
}

/// A map of service names to service definitions
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestServices(
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest_btree_map_alphanum_keys::<ManifestServiceDescriptor>(10, 3)"
        )
    )]
    pub(crate) BTreeMap<String, ManifestServiceDescriptor>,
);

impl ManifestServices {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

impl_into_inner!(ManifestServices, BTreeMap<String, ManifestServiceDescriptor>);

/// The definition of a service in a manifest
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestServiceDescriptor {
    /// The command to run to start the service
    pub command: String,
    /// Service-specific environment variables
    pub vars: Option<ManifestVariables>,
    /// Whether the service spawns a background process (daemon)
    // TODO: This option _requires_ the shutdown command, so we'll need to add
    //       that explanation to the manifest.toml docs and service mgmt guide
    pub is_daemon: Option<bool>,
    /// How to shut down the service
    pub shutdown: Option<ManifestServiceShutdown>,
    /// Systems to allow running the service on
    pub systems: Option<Vec<System>>,
}

impl ManifestServices {
    pub fn validate(&self) -> Result<(), ServiceError> {
        let mut bad_services = vec![];
        for (name, desc) in self.0.iter() {
            let daemonizes = desc.is_daemon.is_some_and(|_self| _self);
            let has_shutdown_cmd = desc.shutdown.is_some();
            if daemonizes && !has_shutdown_cmd {
                bad_services.push(name.clone());
            }
        }
        let list = bad_services
            .into_iter()
            .map(|name| format!("- {name}"))
            .join("\n");
        if list.is_empty() {
            Ok(())
        } else {
            let msg = formatdoc! {"
                Services that spawn daemon processes must supply a shutdown command.

                The following services did not specify a shutdown command:
                {list}
            "};
            Err(ServiceError::InvalidConfig(msg))
        }
    }

    /// Create a new [ManifestServices] instance with services
    /// for systems other than `system` filtered out.
    ///
    /// Clone the services rather than filter in place
    /// to avoid accidental mutation of the original in memory manifest/lockfile.
    pub fn copy_for_system(&self, system: &System) -> Self {
        let mut services = BTreeMap::new();
        for (name, desc) in self.0.iter() {
            if desc
                .systems
                .as_ref()
                .map_or(true, |systems| systems.contains(system))
            {
                services.insert(name.clone(), desc.clone());
            }
        }
        ManifestServices(services)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestServiceShutdown {
    /// What command to run to shut down the service
    pub command: String,
}

/// A map of package ids to package build descriptors
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestBuild(
    #[cfg_attr(
        test,
        proptest(strategy = "proptest_btree_map_alphanum_keys::<ManifestBuildDescriptor>(10, 3)")
    )]
    pub(crate) BTreeMap<String, ManifestBuildDescriptor>,
);

impl_into_inner!(ManifestBuild, BTreeMap<String,ManifestBuildDescriptor>);

impl ManifestBuild {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

/// The definition of a package built from within the environment
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestBuildDescriptor {
    /// The command to run to build a package.
    pub command: String,
    /// Files to explicitly include in the build result.
    pub files: Option<Vec<String>>,
    /// Packages from the 'toplevel' group to include in the closure of the build result.
    pub runtime_packages: Option<Vec<String>>,
    /// Systems to allow running the build.
    pub systems: Option<Vec<System>>,
    /// Sandbox mode for the build.
    pub sandbox: Option<ManifestBuildSandbox>,
    /// The version to assign the package.
    pub version: Option<String>,
    /// A short description of the package that will appear on FloxHub and in
    /// search results.
    pub description: Option<String>,
    /// A license to assign to the package in SPDX format.
    pub license: Option<Vec<String>>,
}

/// The definition of a package built from within the environment
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, derive_more::Display)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub enum ManifestBuildSandbox {
    Off,
    Pure,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestContainerize {
    pub config: Option<ManifestContainerizeConfig>,
}

/// Container config derived from
/// https://github.com/opencontainers/image-spec/blob/main/config.md
///
/// Env and Entrypoint are left out since they interfere with our activation implementation
/// Deprecated and reserved keys are also left out
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestContainerizeConfig {
    /// The username or UID which is a platform-specific structure that allows specific control over which user the process run as.
    /// This acts as a default value to use when the value is not specified when creating a container.
    /// For Linux based systems, all of the following are valid: `user`, `uid`, `user:group`, `uid:gid`, `uid:group`, `user:gid`.
    /// If `group`/`gid` is not specified, the default group and supplementary groups of the given `user`/`uid` in `/etc/passwd` and `/etc/group` from the container are applied.
    /// If `group`/`gid` is specified, supplementary groups from the container are ignored.
    pub user: Option<String>,
    /// A set of ports to expose from a container running this image.
    /// Its keys can be in the format of:
    /// `port/tcp`, `port/udp`, `port` with the default protocol being `tcp` if not specified.
    /// These values act as defaults and are merged with any specified when creating a container.
    pub exposed_ports: Option<BTreeSet<String>>,
    /// Default arguments to the entrypoint of the container.
    /// These values act as defaults and may be replaced by any specified when creating a container.
    /// Flox sets an entrypoint to activate the containerized environment,
    /// and `cmd` is then run inside the activation, similar to
    /// `flox activate -- cmd`.
    pub cmd: Option<Vec<String>>,
    /// A set of directories describing where the process is
    /// likely to write data specific to a container instance.
    pub volumes: Option<BTreeSet<String>>,
    /// Sets the current working directory of the entrypoint process in the container.
    /// This value acts as a default and may be replaced by a working directory specified when creating a container.
    pub working_dir: Option<String>,
    /// This field contains arbitrary metadata for the container.
    /// This property MUST use the [annotation rules](https://github.com/opencontainers/image-spec/blob/main/annotations.md#rules).
    pub labels: Option<BTreeMap<String, String>>,
    /// This field contains the system call signal that will be sent to the container to exit. The signal can be a signal name in the format `SIGNAME`, for instance `SIGKILL` or `SIGRTMIN+3`.
    pub stop_signal: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("no package or group named '{0}' in the manifest")]
    PkgOrGroupNotFound(String),
}

#[cfg(test)]
pub(super) mod test {
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;

    use super::*;

    const CATALOG_MANIFEST: &str = indoc! {r#"
        version = 1
    "#};

    #[test]
    fn catalog_manifest_rejects_unknown_fields() {
        let manifest = formatdoc! {"
            {CATALOG_MANIFEST}

            unknown = 'field'
        "};

        let err = toml_edit::de::from_str::<Manifest>(&manifest)
            .expect_err("manifest.toml should be invalid");

        assert!(
            err.message()
                .starts_with("unknown field `unknown`, expected one of"),
            "unexpected error message: {err}",
        );
    }

    #[test]
    fn catalog_manifest_rejects_unknown_nested_fields() {
        let manifest = formatdoc! {"
            {CATALOG_MANIFEST}

            [options]
            allow.unknown = true
        "};

        let err = toml_edit::de::from_str::<Manifest>(&manifest)
            .expect_err("manifest.toml should be invalid");

        assert!(
            err.message()
                .starts_with("unknown field `unknown`, expected one of"),
            "unexpected error message: {err}",
        );
    }

    #[test]
    fn detect_catalog_manifest() {
        assert!(toml_edit::de::from_str::<Manifest>(CATALOG_MANIFEST).is_ok());
    }

    proptest! {
        #[test]
        fn manifest_round_trip(manifest in any::<Manifest>()) {
            let toml = toml_edit::ser::to_string(&manifest).unwrap();
            let parsed = toml_edit::de::from_str::<Manifest>(&toml).unwrap();
            prop_assert_eq!(manifest, parsed);
        }
    }

    fn has_null_fields(json_str: &str) -> bool {
        type Value = serde_json::Value;
        let json_value: Value = serde_json::from_str(json_str).unwrap();

        // Recursively check if any field in the JSON is `null`
        fn check_for_null(value: &Value) -> bool {
            match value {
                Value::Null => true,
                Value::Object(map) => map.values().any(check_for_null),
                Value::Array(arr) => arr.iter().any(check_for_null),
                _ => false,
            }
        }

        check_for_null(&json_value)
    }

    // Null fields rendered into the lockfile cause backwards-compatibility issues for new fields.
    proptest! {
        #[test]
        fn manifest_does_not_serialize_null_fields(manifest in any::<Manifest>()) {
            let json_str = serde_json::to_string_pretty(&manifest).unwrap();
            prop_assert!(!has_null_fields(&json_str), "json: {}", &json_str);
        }
    }

    #[test]
    fn parses_build_section() {
        let build_manifest = indoc! {r#"
            version = 1
            [build]
            test.command = 'hello'

        "#};

        let parsed = toml_edit::de::from_str::<Manifest>(build_manifest).unwrap();

        assert_eq!(
            parsed.build,
            ManifestBuild(
                [("test".to_string(), ManifestBuildDescriptor {
                    command: "hello".to_string(),
                    runtime_packages: None,
                    files: None,
                    systems: None,
                    sandbox: None,
                    version: None,
                    description: None,
                    license: None,
                })]
                .into()
            )
        );
    }

    #[test]
    fn filter_services_by_system() {
        let manifest = indoc! {r#"
            version = 1
            [services]
            postgres.command = "postgres"
            mysql.command = "mysql"
            mysql.systems = ["x86_64-linux", "aarch64-linux"]
            redis.command = "redis"
            redis.systems = ["aarch64-linux"]
        "#};

        let parsed = toml_edit::de::from_str::<Manifest>(manifest).unwrap();

        assert_eq!(parsed.services.inner().len(), 3, "{:?}", parsed.services);

        let filtered = parsed.services.copy_for_system(&"x86_64-linux".to_string());
        assert_eq!(filtered.inner().len(), 2, "{:?}", filtered);
        assert!(filtered.inner().contains_key("postgres"));
        assert!(filtered.inner().contains_key("mysql"));

        let filtered = parsed
            .services
            .copy_for_system(&"aarch64-darwin".to_string());
        assert_eq!(filtered.inner().len(), 1, "{:?}", filtered);
        assert!(filtered.inner().contains_key("postgres"));
    }
}
