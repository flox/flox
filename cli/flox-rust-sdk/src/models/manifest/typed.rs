use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;

use flox_core::activate::mode::ActivateMode;
#[cfg(test)]
use flox_test_utils::proptest::{
    alphanum_and_whitespace_string,
    alphanum_string,
    btree_map_strategy,
    optional_btree_map,
    optional_btree_set,
    optional_string,
    optional_vec_of_strings,
    vec_of_strings,
};
use indoc::formatdoc;
use itertools::Itertools;
#[cfg(test)]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use systemd::unit::ServiceUnit;

use super::raw::RawManifest;
use crate::data::System;
use crate::models::environment_ref::RemoteEnvironmentRef;
use crate::providers::services::process_compose::ServiceError;

pub(crate) const DEFAULT_GROUP_NAME: &str = "toplevel";
pub const DEFAULT_PRIORITY: u64 = 5;

/// An interface codifying how to access types that are just semantic wrappers
/// around inner types. This impl may be generated with a macro.
pub trait Inner {
    type Inner;

    fn inner(&self) -> &Self::Inner;
    fn inner_mut(&mut self) -> &mut Self::Inner;
    fn into_inner(self) -> Self::Inner;
}

/// A macro that generates a `Inner` impl.
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

pub(crate) use impl_into_inner;

/// An interface for the type of function that serde's skip_serializing_if
/// method takes.
pub(crate) trait SkipSerializing {
    fn skip_serializing(&self) -> bool;
}

/// Not meant for writing manifest files, only for reading them.
/// Modifications should be made using `manifest::raw`.

// We use `skip_serializing_none` and `skip_serializing_if` throughout to reduce
// the size of the lockfile and improve backwards compatibility when we
// introduce fields.
//
// It would be better if we could deny_unknown_fields when we're deserializing
// the user provided manifest but allow unknown fields when deserializing the
// lockfile, but that doesn't seem worth the effort at the moment.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: ManifestVersion,
    /// The packages to install in the form of a map from install_id
    /// to package descriptor.
    #[serde(default)]
    #[serde(skip_serializing_if = "Install::skip_serializing")]
    pub install: Install,
    /// Variables that are exported to the shell environment upon activation.
    #[serde(default)]
    #[serde(skip_serializing_if = "Vars::skip_serializing")]
    pub vars: Vars,
    /// Hooks that are run at various times during the lifecycle of the manifest
    /// in a known shell environment.
    #[serde(default)]
    pub hook: Option<Hook>,
    /// Profile scripts that are run in the user's shell upon activation.
    #[serde(default)]
    pub profile: Option<Profile>,
    /// Options that control the behavior of the manifest.
    #[serde(default)]
    pub options: Options,
    /// Service definitions
    #[serde(default)]
    #[serde(skip_serializing_if = "Services::skip_serializing")]
    pub services: Services,
    /// Package build definitions
    #[serde(default)]
    #[serde(skip_serializing_if = "Build::skip_serializing")]
    pub build: Build,
    #[serde(default)]
    pub containerize: Option<Containerize>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Include::skip_serializing")]
    pub include: Include,
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
    ) -> Option<PackageDescriptorCatalog> {
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

    /// Resolve "loose" package references (e.g. pkg-paths),
    /// to `install_ids` if unambiguous
    /// so that installation references remain valid for other package operations.
    pub fn get_install_ids(&self, packages: Vec<String>) -> Result<Vec<String>, ManifestError> {
        let mut install_ids = Vec::new();
        for pkg in packages {
            // User passed an install id directly
            if self.install.inner().contains_key(&pkg) {
                install_ids.push(pkg);
                continue;
            }

            // User passed a package path to uninstall
            // To support version constraints, we match the provided value against
            // `<pkg-path>` and `<pkg-path>@<version>`.
            let matching_iids_by_pkg_path = self
                .install
                .inner()
                .iter()
                .filter(|(_iid, descriptor)| {
                    // Find matching pkg-paths and select for uninstall

                    // If the descriptor is not a catalog descriptor, skip.
                    // flakes descriptors are only matched by install_id.
                    let ManifestPackageDescriptor::Catalog(des) = descriptor else {
                        return false;
                    };

                    // Select if the descriptor's pkg_path matches the user's input
                    if des.pkg_path == pkg {
                        return true;
                    }

                    // Select if the descriptor matches the user's input when the version is included
                    // Future: if we want to allow uninstalling a specific outputs as well,
                    //         parsing of uninstall specs will need to be more sophisticated.
                    //         For now going with a simple check for pkg-path@version.
                    if let Some(version) = &des.version {
                        format!("{}@{}", des.pkg_path, version) == pkg
                    } else {
                        false
                    }
                })
                .map(|(iid, _)| iid.to_owned())
                .collect::<Vec<String>>();

            // Extend the install_ids with the matching install id from pkg-path
            match matching_iids_by_pkg_path.len() {
                0 => return Err(ManifestError::PackageNotFound(pkg)),
                // if there is only one package with the given pkg-path, uninstall it
                1 => install_ids.extend(matching_iids_by_pkg_path),
                // if there are multiple packages with the given pkg-path, ask for a specific install id
                _ => {
                    return Err(ManifestError::MultiplePackagesMatch(
                        pkg,
                        matching_iids_by_pkg_path,
                    ));
                },
            }
        }
        Ok(install_ids)
    }
}

impl FromStr for Manifest {
    type Err = toml_edit::de::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RawManifest::from_str(s)?.to_typed()
    }
}

pub(crate) fn pkg_descriptors_in_toplevel_group(
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> Vec<(String, ManifestPackageDescriptor)> {
    descriptors
        .iter()
        .filter(|(_, desc)| {
            let ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog { pkg_group, .. }) =
                desc
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
            let ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog { pkg_group, .. }) =
                desc
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

    let ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog { pkg_group, .. }) = descriptor
    else {
        return Ok(None);
    };

    let Some(group) = pkg_group else {
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
    let self_in_toplevel_group = pkgs.iter().any(|(id, _)| id == pkg);
    let other_toplevel_packages_exist = pkgs.iter().any(|(id, _)| id != pkg);
    Ok(self_in_toplevel_group && other_toplevel_packages_exist)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct ManifestVersion(u8);

impl Default for ManifestVersion {
    fn default() -> Self {
        Self(1)
    }
}

#[cfg(test)]
impl Arbitrary for ManifestVersion {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
        prop_oneof!(Just(ManifestVersion(1)), Just(ManifestVersion(2)),).boxed()
    }
}

impl_into_inner!(ManifestVersion, u8);

impl From<u8> for ManifestVersion {
    fn from(value: u8) -> Self {
        ManifestVersion(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Install(
    #[cfg_attr(
        test,
        proptest(strategy = "btree_map_strategy::<ManifestPackageDescriptor>(10, 3)")
    )]
    pub(crate) BTreeMap<String, ManifestPackageDescriptor>,
);

impl SkipSerializing for Install {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

impl_into_inner!(Install, BTreeMap<String, ManifestPackageDescriptor>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
// todo: this can make the error messages less clear and might call for a custom (de)serialize impl
#[serde(
    untagged,
    expecting = "Expected either a catalog package descriptor, a flake installable or a store path.
See https://flox.dev/docs/reference/command-reference/manifest.toml/#package-descriptors for more information."
)]
pub enum ManifestPackageDescriptor {
    Catalog(PackageDescriptorCatalog),
    FlakeRef(PackageDescriptorFlake),
    StorePath(PackageDescriptorStorePath),
}

impl ManifestPackageDescriptor {
    /// Check if the package descriptor is from a custom catalog.
    /// Only Catalog type descriptors are considered to be from a custom catalog.
    pub(crate) fn is_from_custom_catalog(&self) -> bool {
        match self {
            ManifestPackageDescriptor::Catalog(pkg) => super::raw::is_custom_package(&pkg.pkg_path),
            _ => false,
        }
    }
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
    pub fn unwrap_catalog_descriptor(self) -> Option<PackageDescriptorCatalog> {
        match self {
            ManifestPackageDescriptor::Catalog(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_catalog_descriptor_ref(&self) -> Option<&PackageDescriptorCatalog> {
        match self {
            ManifestPackageDescriptor::Catalog(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn unwrap_flake_descriptor(self) -> Option<PackageDescriptorFlake> {
        match self {
            ManifestPackageDescriptor::FlakeRef(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_flake_descriptor_ref(&self) -> Option<&PackageDescriptorFlake> {
        match self {
            ManifestPackageDescriptor::FlakeRef(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn unwrap_store_path_descriptor(self) -> Option<PackageDescriptorStorePath> {
        match self {
            ManifestPackageDescriptor::StorePath(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_store_path_descriptor_ref(&self) -> Option<&PackageDescriptorStorePath> {
        match self {
            ManifestPackageDescriptor::StorePath(descriptor) => Some(descriptor),
            _ => None,
        }
    }
}

impl From<&PackageDescriptorCatalog> for ManifestPackageDescriptor {
    fn from(val: &PackageDescriptorCatalog) -> Self {
        ManifestPackageDescriptor::Catalog(val.clone())
    }
}

impl From<PackageDescriptorCatalog> for ManifestPackageDescriptor {
    fn from(val: PackageDescriptorCatalog) -> Self {
        ManifestPackageDescriptor::Catalog(val)
    }
}

impl From<&PackageDescriptorFlake> for ManifestPackageDescriptor {
    fn from(val: &PackageDescriptorFlake) -> Self {
        ManifestPackageDescriptor::FlakeRef(val.clone())
    }
}

impl From<PackageDescriptorFlake> for ManifestPackageDescriptor {
    fn from(val: PackageDescriptorFlake) -> Self {
        ManifestPackageDescriptor::FlakeRef(val)
    }
}

impl From<&PackageDescriptorStorePath> for ManifestPackageDescriptor {
    fn from(val: &PackageDescriptorStorePath) -> Self {
        ManifestPackageDescriptor::StorePath(val.clone())
    }
}

impl From<PackageDescriptorStorePath> for ManifestPackageDescriptor {
    fn from(val: PackageDescriptorStorePath) -> Self {
        ManifestPackageDescriptor::StorePath(val)
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct PackageDescriptorCatalog {
    #[cfg_attr(test, proptest(strategy = "alphanum_string(5)"))]
    pub pkg_path: String,
    #[cfg_attr(test, proptest(strategy = "optional_string(5)"))]
    pub pkg_group: Option<String>,
    #[cfg_attr(test, proptest(strategy = "proptest::option::of(0..10u64)"))]
    pub priority: Option<u64>,
    #[cfg_attr(test, proptest(strategy = "optional_string(5)"))]
    pub version: Option<String>,
    #[cfg_attr(test, proptest(strategy = "optional_vec_of_strings(3, 4)"))]
    pub systems: Option<Vec<System>>,
    pub outputs: Option<SelectedOutputs>,
}

impl PackageDescriptorCatalog {
    /// Check if two package descriptors should have the same resolution.
    /// This is used to determine if a package needs to be re-resolved
    /// in the presence of an existing lock.
    ///
    /// * Descriptors are resolved per system,
    ///   changing the supported systems does not invalidate _existing_ resolutions.
    /// * Priority is not used in resolution, so it is ignored.
    pub(super) fn invalidates_existing_resolution(&self, other: &Self) -> bool {
        // unpack to avoid forgetting to update this method when new fields are added
        let PackageDescriptorCatalog {
            pkg_path,
            pkg_group,
            version,
            systems: _,
            priority: _,
            outputs: _,
        } = self;

        pkg_path != &other.pkg_path || pkg_group != &other.pkg_group || version != &other.version
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct PackageDescriptorFlake {
    #[cfg_attr(test, proptest(strategy = "alphanum_string(5)"))]
    pub flake: String,
    #[cfg_attr(test, proptest(strategy = "proptest::option::of(0..10u64)"))]
    pub(crate) priority: Option<u64>,
    #[cfg_attr(test, proptest(strategy = "optional_vec_of_strings(3, 4)"))]
    pub(crate) systems: Option<Vec<System>>,
    pub outputs: Option<SelectedOutputs>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct PackageDescriptorStorePath {
    #[cfg_attr(test, proptest(strategy = "alphanum_string(5)"))]
    pub(crate) store_path: String,
    #[cfg_attr(test, proptest(strategy = "optional_vec_of_strings(3, 4)"))]
    pub(crate) systems: Option<Vec<System>>,
    #[cfg_attr(test, proptest(strategy = "proptest::option::of(0..10u64)"))]
    pub(crate) priority: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[serde(untagged)]
#[serde(deny_unknown_fields)]
pub enum SelectedOutputs {
    All(AllSentinel),
    Specific(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AllSentinel {
    All,
}

#[cfg(test)]
impl Arbitrary for SelectedOutputs {
    type Parameters = ();
    type Strategy = BoxedStrategy<SelectedOutputs>;

    fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
        prop_oneof!(
            Just(SelectedOutputs::All(AllSentinel::All)),
            vec_of_strings(3, 4).prop_map(SelectedOutputs::Specific)
        )
        .boxed()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Vars(
    #[cfg_attr(test, proptest(strategy = "btree_map_strategy::<String>(5, 3)"))]
    pub(crate)  BTreeMap<String, String>,
);

impl SkipSerializing for Vars {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

impl_into_inner!(Vars, BTreeMap<String, String>);

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct Hook {
    /// A script that is run at activation time,
    /// in a flox provided bash shell
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) on_activate: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// When defined, this hook is run by _all_ shells upon activation
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) common: Option<String>,
    /// When defined, this hook is run upon activation in a bash shell
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) bash: Option<String>,
    /// When defined, this hook is run upon activation in a zsh shell
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) zsh: Option<String>,
    /// When defined, this hook is run upon activation in a fish shell
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) fish: Option<String>,
    /// When defined, this hook is run upon activation in a tcsh shell
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) tcsh: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// A list of systems that each package is resolved for.
    #[cfg_attr(test, proptest(strategy = "optional_vec_of_strings(3, 4)"))]
    pub systems: Option<Vec<System>>,
    /// Options that control what types of packages are allowed.
    #[serde(default)]
    #[serde(skip_serializing_if = "Allows::skip_serializing")]
    pub allow: Allows,
    /// Options that control how semver versions are resolved.
    #[serde(default)]
    #[serde(skip_serializing_if = "SemverOptions::skip_serializing")]
    pub semver: SemverOptions,
    /// Whether to detect CUDA devices and libs during activation.
    // TODO: Migrate to `ActivateOptions`.
    pub cuda_detection: Option<bool>,
    /// Options that control the behavior of activations.
    #[serde(default)]
    #[serde(skip_serializing_if = "ActivateOptions::skip_serializing")]
    pub activate: ActivateOptions,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct Allows {
    /// Whether to allow packages that are marked as `unfree`
    pub unfree: Option<bool>,
    /// Whether to allow packages that are marked as `broken`
    pub broken: Option<bool>,
    /// A list of license descriptors that are allowed
    #[serde(default)]
    #[cfg_attr(test, proptest(strategy = "optional_vec_of_strings(3, 4)"))]
    pub licenses: Option<Vec<String>>,
}

impl SkipSerializing for Allows {
    fn skip_serializing(&self) -> bool {
        // Destructuring here prevents us from missing new fields if they're
        // added in the future.
        let Allows {
            unfree,
            broken,
            licenses,
        } = self;
        unfree.is_none() && broken.is_none() && licenses.is_none()
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct SemverOptions {
    /// Whether to allow pre-release versions when resolving
    #[serde(default)]
    pub allow_pre_releases: Option<bool>,
}

impl SkipSerializing for SemverOptions {
    fn skip_serializing(&self) -> bool {
        // Destructuring here prevents us from missing new fields if they're
        // added in the future.
        let SemverOptions { allow_pre_releases } = self;
        allow_pre_releases.is_none()
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ActivateOptions {
    pub mode: Option<ActivateMode>,
}

impl SkipSerializing for ActivateOptions {
    /// Don't write a struct of None's into the lockfile but also don't
    /// explicitly check fields which we might forget to update.
    fn skip_serializing(&self) -> bool {
        self == &ActivateOptions::default()
    }
}

/// A map of service names to service definitions
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Services(
    #[cfg_attr(
        test,
        proptest(strategy = "btree_map_strategy::<ServiceDescriptor>(5, 3)")
    )]
    pub(crate) BTreeMap<String, ServiceDescriptor>,
);

impl SkipSerializing for Services {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

impl_into_inner!(Services, BTreeMap<String, ServiceDescriptor>);

/// The definition of a service in a manifest
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ServiceDescriptor {
    /// The command to run to start the service
    #[cfg_attr(test, proptest(strategy = "alphanum_string(3)"))]
    pub command: String,
    /// Service-specific environment variables
    pub vars: Option<Vars>,
    /// Whether the service spawns a background process (daemon)
    // TODO: This option _requires_ the shutdown command, so we'll need to add
    //       that explanation to the manifest.toml docs and service mgmt guide
    pub is_daemon: Option<bool>,
    /// How to shut down the service
    pub shutdown: Option<ServiceShutdown>,

    /// Additional manual config of the systemd service generated for persistent services
    #[cfg_attr(test, proptest(strategy = "test::service_unit_with_none_fields()"))]
    pub systemd: Option<ServiceUnit>,

    /// Systems to allow running the service on
    #[cfg_attr(test, proptest(strategy = "optional_vec_of_strings(3, 4)"))]
    pub systems: Option<Vec<System>>,
}

impl Services {
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
                .is_none_or(|systems| systems.contains(system))
            {
                services.insert(name.clone(), desc.clone());
            }
        }
        Services(services)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ServiceShutdown {
    /// What command to run to shut down the service
    #[cfg_attr(test, proptest(strategy = "alphanum_string(3)"))]
    pub command: String,
}

/// A map of package ids to package build descriptors
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Build(
    #[cfg_attr(
        test,
        proptest(strategy = "btree_map_strategy::<BuildDescriptor>(5, 3)")
    )]
    pub(crate) BTreeMap<String, BuildDescriptor>,
);

impl_into_inner!(Build, BTreeMap<String,BuildDescriptor>);

impl SkipSerializing for Build {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

/// The definition of a package built from within the environment
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct BuildDescriptor {
    /// The command to run to build a package.
    #[cfg_attr(test, proptest(strategy = "alphanum_string(3)"))]
    pub command: String,
    /// Packages from the 'toplevel' group to include in the closure of the build result.
    #[cfg_attr(test, proptest(strategy = "optional_vec_of_strings(3, 4)"))]
    pub runtime_packages: Option<Vec<String>>,
    /// Sandbox mode for the build.
    pub sandbox: Option<BuildSandbox>,
    /// The version to assign the package.
    pub version: Option<BuildVersion>,
    /// A short description of the package that will appear on FloxHub and in
    /// search results.
    #[cfg_attr(test, proptest(strategy = "optional_string(3)"))]
    pub description: Option<String>,
    /// A license to assign to the package in SPDX format.
    #[cfg_attr(test, proptest(strategy = "optional_vec_of_strings(3, 4)"))]
    pub license: Option<Vec<String>>,
}

/// The definition of a package built from within the environment
#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, derive_more::Display, JsonSchema,
)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub enum BuildSandbox {
    Off,
    Pure,
}

/// The definition of a package built from within the environment
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case", untagged)]
pub enum BuildVersion {
    Pure(#[cfg_attr(test, proptest(strategy = "alphanum_string(3)"))] String),
    File {
        file: PathBuf,
    },
    Command {
        #[cfg_attr(test, proptest(strategy = "alphanum_string(3)"))]
        command: String,
    },
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct Containerize {
    pub config: Option<ContainerizeConfig>,
}

/// Container config derived from
/// https://github.com/opencontainers/image-spec/blob/main/config.md
///
/// Env and Entrypoint are left out since they interfere with our activation implementation
/// Deprecated and reserved keys are also left out
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ContainerizeConfig {
    /// The username or UID which is a platform-specific structure that allows specific control over which user the process run as.
    /// This acts as a default value to use when the value is not specified when creating a container.
    /// For Linux based systems, all of the following are valid: `user`, `uid`, `user:group`, `uid:gid`, `uid:group`, `user:gid`.
    /// If `group`/`gid` is not specified, the default group and supplementary groups of the given `user`/`uid` in `/etc/passwd` and `/etc/group` from the container are applied.
    /// If `group`/`gid` is specified, supplementary groups from the container are ignored.
    #[cfg_attr(test, proptest(strategy = "optional_string(3)"))]
    pub user: Option<String>,
    /// A set of ports to expose from a container running this image.
    /// Its keys can be in the format of:
    /// `port/tcp`, `port/udp`, `port` with the default protocol being `tcp` if not specified.
    /// These values act as defaults and are merged with any specified when creating a container.
    #[cfg_attr(test, proptest(strategy = "optional_btree_set(3, 4)"))]
    pub exposed_ports: Option<BTreeSet<String>>,
    /// Default arguments to the entrypoint of the container.
    /// These values act as defaults and may be replaced by any specified when creating a container.
    /// Flox sets an entrypoint to activate the containerized environment,
    /// and `cmd` is then run inside the activation, similar to
    /// `flox activate -- cmd`.
    #[cfg_attr(test, proptest(strategy = "optional_vec_of_strings(3, 4)"))]
    pub cmd: Option<Vec<String>>,
    /// A set of directories describing where the process is
    /// likely to write data specific to a container instance.
    #[cfg_attr(test, proptest(strategy = "optional_btree_set(3, 4)"))]
    pub volumes: Option<BTreeSet<String>>,
    /// Sets the current working directory of the entrypoint process in the container.
    /// This value acts as a default and may be replaced by a working directory specified when creating a container.
    #[cfg_attr(test, proptest(strategy = "optional_string(3)"))]
    pub working_dir: Option<String>,
    /// This field contains arbitrary metadata for the container.
    /// This property MUST use the [annotation rules](https://github.com/opencontainers/image-spec/blob/main/annotations.md#rules).
    #[cfg_attr(test, proptest(strategy = "optional_btree_map::<String>(3, 4)"))]
    pub labels: Option<BTreeMap<String, String>>,
    /// This field contains the system call signal that will be sent to the container to exit. The signal can be a signal name in the format `SIGNAME`, for instance `SIGKILL` or `SIGRTMIN+3`.
    #[cfg_attr(test, proptest(strategy = "optional_string(3)"))]
    pub stop_signal: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("no package or group named '{0}' in the manifest")]
    PkgOrGroupNotFound(String),
    #[error("no package named '{0}' in the manifest")]
    PackageNotFound(String),
    #[error(
        "multiple packages match '{0}', please specify an install id from possible matches: {1:?}"
    )]
    MultiplePackagesMatch(String, Vec<String>),
}

/// The section where users can declare dependencies on other environments.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct Include {
    #[serde(default)]
    pub environments: Vec<IncludeDescriptor>,
}

impl SkipSerializing for Include {
    fn skip_serializing(&self) -> bool {
        self.environments.is_empty()
    }
}

/// The structure for how a user is able to declare a dependency on an environment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
#[serde(
    untagged,
    expecting = "expected { dir = <dir>, [name = <name>] } OR { remote = <owner/name>, [name = <name>] }"
)]
pub enum IncludeDescriptor {
    Local {
        /// The directory where the environment is located.
        dir: PathBuf,
        /// A name similar to an install ID that a user could use to specify
        /// the environment on the command line e.g. for upgrades, or in an
        /// error message.
        #[cfg_attr(test, proptest(strategy = "optional_string(5)"))]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Remote {
        /// The remote environment reference in the form `owner/name`.
        #[serde(alias = "reference")]
        remote: RemoteEnvironmentRef,
        /// A name similar to an install ID that a user could use to specify
        /// the environment on the command line e.g. for upgrades, or in an
        /// error message.
        #[cfg_attr(test, proptest(strategy = "optional_string(5)"))]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, proptest(strategy = "proptest::option::of(0..10usize)"))]
        generation: Option<usize>,
    },
}

impl Display for IncludeDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IncludeDescriptor::Local { dir, name, .. } => {
                write!(f, "{}", name.as_deref().unwrap_or(&dir.to_string_lossy()))
            },
            IncludeDescriptor::Remote { remote, name, .. } => {
                write!(f, "{}", name.as_deref().unwrap_or(&remote.to_string()))
            },
        }
    }
}

#[cfg(test)]
pub mod test {
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;

    use super::*;

    const CATALOG_MANIFEST: &str = indoc! {r#"
        version = 1
    "#};

    /// Generate a single ServiceUnit with just enough fields to test `skip_serializing_none`
    /// Generating more than 1(!) value with proptest,
    /// increases the runtime of `proptest!`s to the point that we exhausted our stack space in CI
    pub(super) fn service_unit_with_none_fields() -> impl Strategy<Value = Option<ServiceUnit>> {
        Just(Some(ServiceUnit {
            unit: Some(systemd::unit::Unit {
                ..Default::default()
            }),
            service: Some(systemd::unit::Service {
                ..Default::default()
            }),
        }))
    }

    // Generate a Manifest that has empty install and include sections
    pub fn manifest_without_install_or_include() -> impl Strategy<Value = Manifest> {
        (
            any::<ManifestVersion>(),
            any::<Vars>(),
            any::<Option<Hook>>(),
            any::<Option<Profile>>(),
            any::<Options>(),
            any::<Services>(),
            any::<Build>(),
            any::<Option<Containerize>>(),
        )
            .prop_map(
                |(version, vars, hook, profile, options, services, build, containerize)| Manifest {
                    version,
                    install: Install::default(),
                    vars,
                    hook,
                    profile,
                    options,
                    services,
                    build,
                    containerize,
                    include: Include::default(),
                },
            )
    }

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

        #[test]
        fn manifest_from_str_round_trip(manifest in any::<Manifest>()) {
            let toml = toml_edit::ser::to_string(&manifest).unwrap();
            let parsed = Manifest::from_str(&toml).unwrap();
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

    // A serialized manifest shouldn't contain any tables that aren't specified
    // or required, with the exception of `options` which is fiddly to implement
    // `skip_serializing_if` for such a mixture of fields.
    //
    // This makes the lockfile tidier and improve cross-version compatibility.
    // It doesn't affect the presentation of composed manifests because `flox
    // list` uses a different serializer.
    #[test]
    fn serialize_omits_unspecified_fields() {
        let manifest = Manifest::default();
        let expected = indoc! {r#"
            version = 1

            [options]
        "#};

        let actual = toml_edit::ser::to_string_pretty(&manifest).unwrap();
        assert_eq!(actual, expected);
    }

    // If a user specifies an uncommented `[hook]` or `[profile]` table without
    // any contents, like the manifest template does, then we preserve that in
    // the serialized output.
    #[test]
    fn serialize_preserves_explicitly_empty_tables() {
        let manifest = Manifest {
            hook: Some(Hook::default()),
            profile: Some(Profile::default()),
            ..Default::default()
        };
        let expected = indoc! {r#"
            version = 1

            [hook]

            [profile]

            [options]
        "#};

        let actual = toml_edit::ser::to_string_pretty(&manifest).unwrap();
        assert_eq!(actual, expected);
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
            Build(
                [("test".to_string(), BuildDescriptor {
                    command: "hello".to_string(),
                    runtime_packages: None,
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
    fn parses_version() {
        #[derive(Deserialize)]
        struct VersionWrap {
            version: BuildVersion,
        }
        let parse =
            |version| toml_edit::de::from_str::<VersionWrap>(version).map(|wrap| wrap.version);

        assert_eq!(
            parse("version = '1.2.3'"),
            Ok(BuildVersion::Pure("1.2.3".into()))
        );
        assert_eq!(
            parse("version.file = 'FILE'"),
            Ok(BuildVersion::File {
                file: "FILE".into()
            })
        );
        assert_eq!(
            parse("version.command = 'command'"),
            Ok(BuildVersion::Command {
                command: "command".into()
            })
        );
        assert!(parse("other = 'wont parse'").is_err())
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

    #[test]
    fn parses_include_section_manifest() {
        let manifest = indoc! {r#"
            version = 1

            [include]
            environments = [
                { dir = "../foo", name = "bar" },
                { remote = "owner/repo", name = "baz" },
                # reference alias for remote
                { reference = "owner/repo", name = "bap" },
            ]
        "#};
        let parsed = toml_edit::de::from_str::<Manifest>(manifest).unwrap();

        assert_eq!(parsed.include.environments, vec![
            IncludeDescriptor::Local {
                dir: PathBuf::from("../foo"),
                name: Some("bar".to_string()),
            },
            IncludeDescriptor::Remote {
                remote: RemoteEnvironmentRef::new("owner", "repo").unwrap(),
                name: Some("baz".to_string()),
                generation: None,
            },
            IncludeDescriptor::Remote {
                remote: RemoteEnvironmentRef::new("owner", "repo").unwrap(),
                name: Some("bap".to_string()),
                generation: None,
            },
        ]);
    }

    /// Generates a mock `TypedManifest` for testing purposes.
    /// This function is designed to simplify the creation of test data by
    /// generating a `TypedManifest` based on a list of install IDs and
    /// package paths.
    /// # Arguments
    ///
    /// * `entries` - A vector of tuples, where each tuple contains an install
    ///   ID and a package path.
    ///
    /// # Returns
    ///
    /// * `TypedManifest` - A mock `TypedManifest` containing the provided entries.
    fn generate_mock_manifest(entries: Vec<(&str, &str)>) -> Manifest {
        let mut typed_manifest_mock = Manifest::default();

        for (test_iid, dotted_package) in entries {
            typed_manifest_mock.install.inner_mut().insert(
                test_iid.to_string(),
                ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog {
                    pkg_path: dotted_package.to_string(),
                    pkg_group: None,
                    priority: None,
                    version: None,
                    systems: None,
                    outputs: None,
                }),
            );
        }

        typed_manifest_mock
    }
    /// Return the install ID if it matches the user input
    #[test]
    fn test_get_install_ids_to_uninstall_by_install_id() {
        let manifest_mock = generate_mock_manifest(vec![("testInstallID", "dotted.package")]);
        let result = manifest_mock
            .get_install_ids(vec!["testInstallID".to_string()])
            .unwrap();
        assert_eq!(result, vec!["testInstallID".to_string()]);
    }

    #[test]
    /// Return the install ID if a pkg-path matches the user input
    fn test_get_install_ids_to_uninstall_by_pkg_path() {
        let manifest_mock = generate_mock_manifest(vec![("testInstallID", "dotted.package")]);
        let result = manifest_mock
            .get_install_ids(vec!["dotted.package".to_string()])
            .unwrap();
        assert_eq!(result, vec!["testInstallID".to_string()]);
    }

    #[test]
    /// Ensure that the install ID takes precedence over pkg-path when both are present
    fn test_get_install_ids_to_uninstall_iid_wins() {
        let manifest_mock = generate_mock_manifest(vec![
            ("testInstallID1", "dotted.package"),
            ("testInstallID2", "dotted.package"),
            ("dotted.package", "dotted.package"),
        ]);

        let result = manifest_mock
            .get_install_ids(vec!["dotted.package".to_string()])
            .unwrap();
        assert_eq!(result, vec!["dotted.package".to_string()]);
    }

    #[test]
    /// Throw an error when multiple packages match by pkg_path and flox can't determine which to uninstall
    fn test_get_install_ids_to_uninstall_multiple_pkg_paths_match() {
        let manifest_mock = generate_mock_manifest(vec![
            ("testInstallID1", "dotted.package"),
            ("testInstallID2", "dotted.package"),
            ("testInstallID3", "dotted.package"),
        ]);
        let result = manifest_mock
            .get_install_ids(vec!["dotted.package".to_string()])
            .unwrap_err();
        assert!(matches!(result, ManifestError::MultiplePackagesMatch(_, _)));
    }

    #[test]
    /// Throw an error if no install ID or pkg-path matches the user input
    fn test_get_install_ids_to_uninstall_pkg_not_found() {
        let manifest_mock = generate_mock_manifest(vec![("testInstallID1", "dotted.package")]);
        let result = manifest_mock
            .get_install_ids(vec!["invalid.packageName".to_string()])
            .unwrap_err();
        assert!(matches!(result, ManifestError::PackageNotFound(_)));
    }

    #[test]
    fn test_get_install_ids_to_uninstall_with_version() {
        let mut manifest_mock = generate_mock_manifest(vec![("testInstallID", "dotted.package")]);

        if let ManifestPackageDescriptor::Catalog(descriptor) = manifest_mock
            .install
            .inner_mut()
            .get_mut("testInstallID")
            .unwrap()
        {
            descriptor.version = Some("1.0".to_string());
        };

        let result = manifest_mock
            .get_install_ids(vec!["dotted.package@1.0".to_string()])
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "testInstallID");
    }

    /// Helper function to create a catalog descriptor for testing
    fn create_catalog_descriptor(pkg_path: &str) -> ManifestPackageDescriptor {
        ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog {
            pkg_path: pkg_path.to_string(),
            pkg_group: None,
            priority: None,
            version: None,
            systems: None,
            outputs: None,
        })
    }

    /// Helper function to create a flake descriptor for testing
    fn create_flake_descriptor(flake: &str) -> ManifestPackageDescriptor {
        ManifestPackageDescriptor::FlakeRef(PackageDescriptorFlake {
            flake: flake.to_string(),
            priority: None,
            systems: None,
            outputs: None,
        })
    }

    /// Helper function to create a store path descriptor for testing
    fn create_store_path_descriptor(store_path: &str) -> ManifestPackageDescriptor {
        ManifestPackageDescriptor::StorePath(PackageDescriptorStorePath {
            store_path: store_path.to_string(),
            systems: None,
            priority: None,
        })
    }

    #[test]
    fn test_is_from_custom_catalog() {
        assert!(!create_catalog_descriptor("hello").is_from_custom_catalog());
        assert!(create_catalog_descriptor("mycatalog/hello").is_from_custom_catalog());

        // Test non-catalog descriptors always return false
        assert!(!create_flake_descriptor("github:owner/repo").is_from_custom_catalog());
        assert!(!create_store_path_descriptor("/nix/store/abc123-hello").is_from_custom_catalog());
    }

    #[test]
    fn deserializes_manifest_with_outputs() {
        let contents_default = r#"
            version = 1

            [install]
            hello.pkg-path = "hello"
        "#;

        let contents_all = r#"
            version = 1

            [install]
            hello.pkg-path = "hello"
            hello.outputs = "all"
        "#;

        let contents_specific = r#"
            version = 1

            [install]
            hello.pkg-path = "hello"
            hello.outputs = ["foo", "bar"]
        "#;

        let _ = Manifest::from_str(contents_default).unwrap();
        let _ = Manifest::from_str(contents_all).unwrap();
        let _ = Manifest::from_str(contents_specific).unwrap();
    }
}
