use flox_core::data::System;
#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::{
    alphanum_string,
    optional_string,
    optional_vec_of_strings,
    vec_of_strings,
};
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::parsed::common::PackageDescriptorStorePath;
use crate::util::is_custom_package;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
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
    pub fn is_from_custom_catalog(&self) -> bool {
        match self {
            ManifestPackageDescriptor::Catalog(pkg) => is_custom_package(&pkg.pkg_path),
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
    pub fn invalidates_existing_resolution(&self, other: &Self) -> bool {
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
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct PackageDescriptorCatalog {
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "alphanum_string(5)")
    )]
    pub pkg_path: String,
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_string(5)")
    )]
    pub pkg_group: Option<String>,
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "proptest::option::of(0..10u64)")
    )]
    pub priority: Option<u64>,
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_string(5)")
    )]
    pub version: Option<String>,
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub systems: Option<Vec<System>>,
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
        } = self;

        pkg_path != &other.pkg_path || pkg_group != &other.pkg_group || version != &other.version
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct PackageDescriptorFlake {
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "alphanum_string(5)")
    )]
    pub flake: String,
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "proptest::option::of(0..10u64)")
    )]
    pub(crate) priority: Option<u64>,
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub(crate) systems: Option<Vec<System>>,
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

#[cfg(any(test, feature = "tests"))]
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
