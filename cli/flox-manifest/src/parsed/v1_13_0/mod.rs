use std::collections::BTreeMap;

use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_core::activate::sandbox_mode::SandboxMode;
use flox_core::data::System;
#[cfg(test)]
use flox_test_utils::proptest::alphanum_and_whitespace_string;
#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::{
    alphanum_string,
    btree_map_strategy,
    optional_string,
    optional_vec_of_strings,
};
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::interfaces::{AsTypedOnlyManifest, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{
    ActivateOptions,
    Allows,
    BuildVersion,
    Containerize,
    Hook,
    Include,
    KnownSchemaVersion,
    SemverOptions,
    Vars,
};
use crate::parsed::v1_10_0::{Install, ManifestPackageDescriptor};
pub use crate::parsed::v1_11_0::MinimumCliVersion;
pub use crate::parsed::v1_12_0::Services;
use crate::parsed::{Inner, SkipSerializing, impl_into_inner};
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
    ///
    /// This is the version-specific [Options] (with `sandbox`), not
    /// `common::Options`.
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

/// Options for V1_13_0.
///
/// This is a version-specific copy of `common::Options` because V1_13_0 adds
/// the `sandbox` field; the other fields (and their leaf types) are identical
/// and continue to live in `parsed::common`.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// A list of systems that each package is resolved for.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
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
    /// The sandbox mode applied when the environment is activated
    /// (`off`, `warn`, `enforce`, or `prompt`). An explicit `--sandbox` flag
    /// on `flox activate` takes precedence over this setting.
    pub sandbox: Option<SandboxMode>,
    /// The sandbox enforcement backend used when the environment is activated
    /// (`libsandbox`, `nix`, `host-native`, `srt`, `oci`, or `libkrun`). The
    /// `--sandbox-backend` flag on `flox activate` and the
    /// `FLOX_SANDBOX_BACKEND` environment variable take precedence over this
    /// setting.
    pub sandbox_backend: Option<SandboxBackend>,
    /// Options that control the behavior of activations.
    #[serde(default)]
    #[serde(skip_serializing_if = "ActivateOptions::skip_serializing")]
    pub activate: ActivateOptions,
}

// Conversion from the common Options, used by the V1_12_0 -> V1_13_0
// migration. The new `sandbox` field defaults to None, which is what makes
// the migration lossless.
impl From<crate::parsed::common::Options> for Options {
    fn from(options: crate::parsed::common::Options) -> Self {
        let crate::parsed::common::Options {
            systems,
            allow,
            semver,
            cuda_detection,
            activate,
        } = options;
        Options {
            systems,
            allow,
            semver,
            cuda_detection,
            sandbox: None,
            sandbox_backend: None,
            activate,
        }
    }
}

/// A map of package ids to package build descriptors.
///
/// This is a version-specific copy of `common::Build` because V1_13_0 adds the
/// `sandbox-allow` field to [BuildDescriptor]; the map is otherwise identical.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct Build(
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "btree_map_strategy::<BuildDescriptor>(5, 3)")
    )]
    pub(crate) BTreeMap<String, BuildDescriptor>,
);

impl_into_inner!(Build, BTreeMap<String, BuildDescriptor>);

impl SkipSerializing for Build {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

/// Sandbox mode for a build.
///
/// This is a version-specific copy of `common::BuildSandbox` because V1_13_0
/// adds the local `warn`, `enforce`, and `prompt` modes. Keeping the new
/// variants out of the common enum is what stops older schema versions (whose
/// `BuildDescriptor` uses `common::BuildSandbox`) from accepting
/// `sandbox = "warn" | "enforce" | "prompt"`.
#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, derive_more::Display, JsonSchema,
)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub enum BuildSandbox {
    Off,
    /// Local build; out-of-closure file access is reported with a warning.
    Warn,
    /// Local build; out-of-closure file access is denied (the build fails).
    Enforce,
    /// Local build; out-of-closure file access is referred to an interactive
    /// prompt (and otherwise denied, as enforce). A transitional mode for
    /// building up a `sandbox-allow` list.
    Prompt,
    Pure,
}

// The older schema versions only knew `off`/`pure`, so the migration maps those
// two through; `warn`/`enforce` cannot appear in a pre-V1_13_0 manifest.
impl From<crate::parsed::common::BuildSandbox> for BuildSandbox {
    fn from(sandbox: crate::parsed::common::BuildSandbox) -> Self {
        match sandbox {
            crate::parsed::common::BuildSandbox::Off => BuildSandbox::Off,
            crate::parsed::common::BuildSandbox::Pure => BuildSandbox::Pure,
        }
    }
}

/// The definition of a package built from within the environment.
///
/// V1_13_0 adds `sandbox-allow`: a list of paths/globs the build is permitted
/// to read from outside its closure without a sandbox warning (or, under
/// `enforce`, without failing). Otherwise identical to `common::BuildDescriptor`.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct BuildDescriptor {
    /// The command to run to build a package.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "alphanum_string(3)")
    )]
    pub command: String,
    /// Packages from the 'toplevel' group to include in the closure of the
    /// build result.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub runtime_packages: Option<Vec<String>>,
    /// Sandbox mode for the build.
    pub sandbox: Option<BuildSandbox>,
    /// Paths or glob patterns the build may read from outside its closure
    /// without the virtual sandbox warning about them (or, under `enforce`,
    /// blocking them). A leading `~/` is expanded to `$HOME`; `*`/`**` are
    /// matched with `fnmatch`. Only meaningful for the local sandbox modes
    /// (`warn`/`enforce`).
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub sandbox_allow: Option<Vec<String>>,
    /// The version to assign the package.
    pub version: Option<BuildVersion>,
    /// A short description of the package that will appear on FloxHub and in
    /// search results.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_string(3)")
    )]
    pub description: Option<String>,
    /// A license to assign to the package in SPDX format.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub license: Option<Vec<String>>,
}

// Conversions from the common types, used by the V1_12_0 -> V1_13_0 migration.
// The new `sandbox_allow` field defaults to None, which is what makes the
// migration lossless.
impl From<crate::parsed::common::BuildDescriptor> for BuildDescriptor {
    fn from(descriptor: crate::parsed::common::BuildDescriptor) -> Self {
        let crate::parsed::common::BuildDescriptor {
            command,
            runtime_packages,
            sandbox,
            version,
            description,
            license,
        } = descriptor;
        BuildDescriptor {
            command,
            runtime_packages,
            sandbox: sandbox.map(Into::into),
            sandbox_allow: None,
            version,
            description,
            license,
        }
    }
}

impl From<crate::parsed::common::Build> for Build {
    fn from(build: crate::parsed::common::Build) -> Self {
        Build(
            build
                .into_inner()
                .into_iter()
                .map(|(id, descriptor)| (id, descriptor.into()))
                .collect(),
        )
    }
}
