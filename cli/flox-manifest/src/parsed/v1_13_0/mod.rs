use std::collections::BTreeMap;

#[cfg(test)]
use flox_test_utils::proptest::alphanum_and_whitespace_string;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::interfaces::{AsTypedOnlyManifest, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{
    Build,
    Containerize,
    Hook,
    Include,
    KnownSchemaVersion,
    Options,
    Vars,
};
use crate::parsed::v1_10_0::{Install, ManifestPackageDescriptor};
pub use crate::parsed::v1_11_0::MinimumCliVersion;
pub use crate::parsed::v1_12_0::Services;
use crate::parsed::{Inner, SkipSerializing};
use crate::{Manifest, ManifestError, Parsed, TypedOnly};

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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct ManifestV1_13_0 {
    /// Which schema version this manifest adheres to.
    ///
    /// Must be a valid Flox CLI version listed in [`KnownSchemaVersion`].
    #[serde(rename = "schema-version")]
    pub schema_version: String,
    /// The minimum CLI version that can activate this environment.
    #[serde(rename = "minimum-cli-version")]
    pub minimum_cli_version: Option<MinimumCliVersion>,
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
impl_pkg_lookup!(crate::parsed::v1_10_0, ManifestV1_13_0);

// You can't derive `Default` because `schema-version` is a `String`,
// which just defaults to an empty string.
impl Default for ManifestV1_13_0 {
    fn default() -> Self {
        Self {
            schema_version: "1.13.0".into(),
            minimum_cli_version: Default::default(),
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

impl AsTypedOnlyManifest for ManifestV1_13_0 {
    fn as_typed_only(&self) -> crate::Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::V1_13_0(self.clone()),
            },
        }
    }
}

impl SchemaVersion for ManifestV1_13_0 {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        KnownSchemaVersion::V1_13_0
    }
}

/// Profile scripts for V1_13_0: adds an optional `deactivate` table holding
/// per-shell scripts to run when the environment is deactivated. The
/// activation fields (`common`, `bash`, `zsh`, `fish`, `tcsh`) are the same
/// as earlier schema versions.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
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
    /// Per-shell scripts to run when the environment is deactivated.
    /// Mirrors the activation fields above; each is optional.
    #[serde(default)]
    pub deactivate: Option<ProfileDeactivate>,
}

/// Deactivation profile scripts. Each field, when defined, is sourced as
/// the user's environment is being torn down — symmetric to the activation
/// scripts on the enclosing [`Profile`].
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct ProfileDeactivate {
    /// Run by all shells when the environment is deactivated.
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) common: Option<String>,
    /// Run upon deactivation in a bash shell.
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) bash: Option<String>,
    /// Run upon deactivation in a zsh shell.
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) zsh: Option<String>,
    /// Run upon deactivation in a fish shell.
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) fish: Option<String>,
    /// Run upon deactivation in a tcsh shell.
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) tcsh: Option<String>,
}
