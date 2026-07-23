use std::collections::BTreeMap;

use flox_core::activate::mode::ActivateMode;
use flox_core::data::System;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::interfaces::{AsTypedOnlyManifest, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{
    Allows,
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
pub use crate::parsed::v1_13_0::{
    Build,
    BuildDescriptor,
    BuildSandbox,
    Profile,
    ProfileDeactivate,
};
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
pub struct ManifestV1_14_0 {
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
impl_pkg_lookup!(crate::parsed::v1_10_0, ManifestV1_14_0);

// You can't derive `Default` because `schema-version` is a `String`,
// which just defaults to an empty string.
impl Default for ManifestV1_14_0 {
    fn default() -> Self {
        Self {
            schema_version: "1.14.0".into(),
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

/// V1_14_0-local `Options`: duplicates `parsed::common::Options` but uses the
/// V1_14_0-local `ActivateOptions` (which adds `add-sbin`). All other leaf
/// types (`Allows`, `SemverOptions`) continue to be imported from
/// `parsed::common`.
///
/// This is duplicated (rather than flattened) because `common::Options`
/// carries `#[serde(deny_unknown_fields)]`, which conflicts with
/// `#[serde(flatten)]`.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct Options {
    /// A list of systems that each package is resolved for.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "flox_test_utils::proptest::optional_vec_of_strings(3, 4)")
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
    /// Options that control the behavior of activations.
    #[serde(default)]
    #[serde(skip_serializing_if = "ActivateOptions::skip_serializing")]
    pub activate: ActivateOptions,
}

/// V1_14_0-local `ActivateOptions`: adds `add-sbin` alongside `mode`.
///
/// The `mode` field continues to reference the shared
/// `flox_core::activate::mode::ActivateMode` enum.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ActivateOptions {
    pub mode: Option<ActivateMode>,
    /// Whether to include the environment's `sbin` directory in PATH when
    /// activating. Defaults to `None` (treated as `false`) so that `sbin`
    /// binaries don't shadow binaries from other packages.
    pub add_sbin: Option<bool>,
}

impl SkipSerializing for ActivateOptions {
    /// Don't write a struct of None's into the lockfile but also don't
    /// explicitly check fields which we might forget to update.
    fn skip_serializing(&self) -> bool {
        self == &ActivateOptions::default()
    }
}

// Conversions from the common types, used by the V1_13_0 -> V1_14_0 migration.
// The new `add_sbin` field defaults to None, which is what makes the
// migration lossless.
impl From<crate::parsed::common::ActivateOptions> for ActivateOptions {
    fn from(activate: crate::parsed::common::ActivateOptions) -> Self {
        let crate::parsed::common::ActivateOptions { mode } = activate;
        ActivateOptions {
            mode,
            add_sbin: None,
        }
    }
}

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
            activate: activate.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::test_helpers::with_latest_schema;

    /// `options.activate.add-sbin = true` parses with v1.14.0.
    #[test]
    fn parses_add_sbin_true() {
        let manifest = with_latest_schema(indoc! {r#"
            [options.activate]
            add-sbin = true
        "#});
        let parsed: ManifestV1_14_0 = toml_edit::de::from_str(&manifest).unwrap();
        assert_eq!(parsed.options.activate, ActivateOptions {
            mode: None,
            add_sbin: Some(true),
        });
    }

    /// Omitting `add-sbin` leaves the field as `None`.
    #[test]
    fn add_sbin_defaults_to_none() {
        let manifest = with_latest_schema("");
        let parsed: ManifestV1_14_0 = toml_edit::de::from_str(&manifest).unwrap();
        assert_eq!(parsed.options.activate, ActivateOptions::default());
    }

    /// A manifest whose `ActivateOptions` is all-default must not emit any
    /// keys into the serialized output.
    #[test]
    fn add_sbin_none_skipped_in_serialization() {
        let opts = ActivateOptions::default();
        assert!(opts.skip_serializing());
    }

    /// A manifest with `add-sbin = Some(false)` still needs to be serialized
    /// so that downstream tooling can distinguish "unset" from "explicitly
    /// off".
    #[test]
    fn add_sbin_explicit_false_is_serialized() {
        let opts = ActivateOptions {
            mode: None,
            add_sbin: Some(false),
        };
        assert!(!opts.skip_serializing());
    }
}
