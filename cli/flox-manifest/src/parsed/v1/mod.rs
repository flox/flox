use std::collections::BTreeMap;

#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::btree_map_strategy;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::interfaces::{AsTypedOnlyManifest, CommonFields, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{
    Build,
    Containerize,
    Hook,
    Include,
    KnownSchemaVersion,
    Options,
    Profile,
    Services,
    Vars,
};
use crate::parsed::{Inner, SkipSerializing, impl_into_inner};
use crate::{Manifest, ManifestError, Parsed, TypedOnly};

mod package_descriptor;
pub use package_descriptor::*;

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
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct ManifestV1 {
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
impl_pkg_lookup!(crate::parsed::v1, ManifestV1);

impl AsTypedOnlyManifest for ManifestV1 {
    fn as_typed_only(&self) -> crate::Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::V1(self.clone()),
            },
        }
    }
}

impl SchemaVersion for ManifestV1 {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        KnownSchemaVersion::V1
    }
}

impl CommonFields for ManifestV1 {
    fn vars(&self) -> &Vars {
        &self.vars
    }

    fn hook(&self) -> Option<&Hook> {
        self.hook.as_ref()
    }

    fn profile(&self) -> Option<&Profile> {
        self.profile.as_ref()
    }

    fn services(&self) -> &Services {
        &self.services
    }

    fn include(&self) -> &Include {
        &self.include
    }

    fn build(&self) -> &Build {
        &self.build
    }

    fn containerize(&self) -> Option<&super::common::Containerize> {
        self.containerize.as_ref()
    }

    fn options(&self) -> &super::common::Options {
        &self.options
    }

    fn vars_mut(&mut self) -> &mut super::common::Vars {
        &mut self.vars
    }

    fn hook_mut(&mut self) -> Option<&mut super::common::Hook> {
        self.hook.as_mut()
    }

    fn profile_mut(&mut self) -> Option<&mut super::common::Profile> {
        self.profile.as_mut()
    }

    fn services_mut(&mut self) -> &mut super::common::Services {
        &mut self.services
    }

    fn include_mut(&mut self) -> &mut super::common::Include {
        &mut self.include
    }

    fn build_mut(&mut self) -> &mut super::common::Build {
        &mut self.build
    }

    fn containerize_mut(&mut self) -> Option<&mut super::common::Containerize> {
        self.containerize.as_mut()
    }

    fn options_mut(&mut self) -> &mut super::common::Options {
        &mut self.options
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, JsonSchema)]
pub struct ManifestVersion(u8);

impl Default for ManifestVersion {
    fn default() -> Self {
        Self(1)
    }
}

#[cfg(any(test, feature = "tests"))]
impl Arbitrary for ManifestVersion {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
        Just(ManifestVersion(1)).boxed()
    }
}

impl_into_inner!(ManifestVersion, u8);

impl From<u8> for ManifestVersion {
    fn from(value: u8) -> Self {
        ManifestVersion(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct Install(
    #[cfg_attr(
        any(test, feature = "tests"),
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

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use super::*;

    // Generate a Manifest that has empty install and include sections
    pub fn manifest_without_install_or_include() -> impl Strategy<Value = ManifestV1> {
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
                |(version, vars, hook, profile, options, services, build, containerize)| {
                    ManifestV1 {
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
                    }
                },
            )
    }
}
