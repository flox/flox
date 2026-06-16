use std::collections::BTreeMap;

#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::optional_string;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::interfaces::{AsTypedOnlyManifest, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{Containerize, Hook, Include, KnownSchemaVersion, Options, Vars};
use crate::parsed::v1_10_0::{Install, ManifestPackageDescriptor};
// Leaf types that V1_14_0 leaves unchanged continue to live in their original
// version modules (per the manifest schema-versioning convention). V1_14_0 only
// adds the top-level `description` field, so everything else is re-used.
pub use crate::parsed::v1_13_0::{
    Build,
    BuildDescriptor,
    BuildSandbox,
    MinimumCliVersion,
    Profile,
    ProfileDeactivate,
    Services,
};
use crate::parsed::{Inner, SkipSerializing};
use crate::{Manifest, ManifestError, Parsed, TypedOnly};

/// Not meant for writing manifest files, only for reading them.
/// Modifications should be made using `manifest::raw`.
///
/// V1_14_0 adds an optional top-level `description` field: a short, one-line
/// summary of what the environment provides. It is surfaced in environment
/// listings, search results, and at the top of `flox info`. Longer-form
/// documentation lives alongside the manifest in `.flox/env/README.md` rather
/// than in the manifest itself.
///
/// We use `skip_serializing_none` and `skip_serializing_if` throughout to reduce
/// the size of the lockfile and improve backwards compatibility when we
/// introduce fields.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct ManifestV1_14_0 {
    /// Which schema version this manifest adheres to.
    ///
    /// Must be a valid Flox CLI version listed in [`KnownSchemaVersion`].
    #[serde(rename = "schema-version")]
    pub schema_version: String,
    /// The minimum CLI version that can activate this environment.
    #[serde(rename = "minimum-cli-version")]
    pub minimum_cli_version: Option<MinimumCliVersion>,
    /// A short, one-line description of what the environment provides.
    ///
    /// Shown in environment listings, search results, and at the top of
    /// `flox info`. Longer documentation belongs in `.flox/env/README.md`.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_string(5)")
    )]
    pub description: Option<String>,
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
    /// Profile scripts that are run in the user's shell upon activation
    /// (and, optionally, upon deactivation).
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
impl_pkg_lookup!(crate::parsed::v1_10_0, ManifestV1_14_0);

// You can't derive `Default` because `schema-version` is a `String`,
// which just defaults to an empty string.
impl Default for ManifestV1_14_0 {
    fn default() -> Self {
        Self {
            schema_version: "1.14.0".into(),
            minimum_cli_version: Default::default(),
            description: Default::default(),
            install: Default::default(),
            vars: Default::default(),
            hook: Default::default(),
            profile: Default::default(),
            options: Default::default(),
            services: Default::default(),
            build: Default::default(),
            containerize: Default::default(),
            include: Default::default(),
        }
    }
}

impl AsTypedOnlyManifest for ManifestV1_14_0 {
    fn as_typed_only(&self) -> crate::Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::V1_14_0(self.clone()),
            },
        }
    }
}

impl SchemaVersion for ManifestV1_14_0 {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        KnownSchemaVersion::V1_14_0
    }
}
