use std::collections::BTreeMap;

use flox_core::activate::mode::ActivateMode;
use flox_core::data::System;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::interfaces::{AsTypedOnlyManifest, CommonFields, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{
    Allows,
    Build,
    Containerize,
    Hook,
    Include,
    KnownSchemaVersion,
    Profile,
    SemverOptions,
    ServiceDescriptor,
    Vars,
};
use crate::parsed::v1_10_0::{Install, ManifestPackageDescriptor};
pub use crate::parsed::v1_11_0::MinimumCliVersion;
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
pub struct ManifestV1_12_0 {
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
impl_pkg_lookup!(crate::parsed::v1_10_0, ManifestV1_12_0);

// You can't derive `Default` because `schema-version` is a `String`,
// which just defaults to an empty string.
impl Default for ManifestV1_12_0 {
    fn default() -> Self {
        Self {
            schema_version: "1.12.0".into(),
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

impl AsTypedOnlyManifest for ManifestV1_12_0 {
    fn as_typed_only(&self) -> crate::Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::V1_12_0(self.clone()),
            },
        }
    }
}

impl SchemaVersion for ManifestV1_12_0 {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        KnownSchemaVersion::V1_12_0
    }
}

impl CommonFields for ManifestV1_12_0 {
    fn vars(&self) -> &Vars {
        &self.vars
    }

    fn hook(&self) -> Option<&Hook> {
        self.hook.as_ref()
    }

    fn profile(&self) -> Option<&Profile> {
        self.profile.as_ref()
    }

    fn services(&self) -> &crate::parsed::common::Services {
        &self.services.service_map
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

    fn systems(&self) -> Option<&Vec<System>> {
        self.options.systems.as_ref()
    }

    fn allows(&self) -> &Allows {
        &self.options.allow
    }

    fn semver_options(&self) -> &SemverOptions {
        &self.options.semver
    }

    fn cuda_detection(&self) -> Option<bool> {
        self.options.cuda_detection
    }

    fn activate_mode(&self) -> Option<&ActivateMode> {
        self.options.activate.mode.as_ref()
    }

    fn activate_add_sbin(&self) -> Option<bool> {
        self.options.activate.add_sbin
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
        &mut self.services.service_map
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

    fn systems_mut(&mut self) -> &mut Option<Vec<System>> {
        &mut self.options.systems
    }

    fn activate_mode_mut(&mut self) -> &mut Option<ActivateMode> {
        &mut self.options.activate.mode
    }

    fn services_auto_start(&self) -> bool {
        self.services.auto_start == Some(true)
    }
}

/// V1_12_0-local `ActivateOptions`: adds `add-sbin` alongside `mode`.
///
/// This is duplicated from `parsed::common::ActivateOptions` (rather than
/// flattened) because `common::ActivateOptions` carries
/// `#[serde(deny_unknown_fields)]`, which conflicts with `#[serde(flatten)]`.
/// The `mode` field continues to reference the shared `common::ActivateMode`
/// enum.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ActivateOptions {
    pub mode: Option<ActivateMode>,
    /// Whether to include `$FLOX_ENV/sbin` in PATH when activating.
    /// Defaults to `None` (treated as `false`) so that `sbin` binaries don't
    /// shadow binaries from other packages.
    pub add_sbin: Option<bool>,
}

impl SkipSerializing for ActivateOptions {
    fn skip_serializing(&self) -> bool {
        // Destructuring here prevents us from missing new fields if they're
        // added in the future.
        let ActivateOptions { mode, add_sbin } = self;
        mode.is_none() && add_sbin.is_none()
    }
}

/// V1_12_0-local `Options`: duplicates `parsed::common::Options` but uses the
/// V1_12_0-local `ActivateOptions` (which adds `add-sbin`). All other leaf
/// types (`Allows`, `SemverOptions`) continue to be imported from
/// `parsed::common`.
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

/// Service configuration for V1_12_0: adds optional auto-start behavior
/// alongside the map of service names to service definitions.
///
/// The `service_map` field is a `parsed::common::Services` (BTreeMap tuple
/// struct) to allow sharing the `CommonFields::services()` accessor across
/// all schema versions without requiring a common trait method that returns
/// a version-specific type.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct Services {
    /// Whether to start all services automatically on `flox activate`.
    /// Can be suppressed with `--no-start-services`.
    #[serde(rename = "auto-start")]
    pub auto_start: Option<bool>,

    /// Map of service names to service definitions.
    ///
    /// Note: `deny_unknown_fields` is NOT on `Services` itself because that
    /// would conflict with `#[serde(flatten)]` here — serde cannot validate
    /// unknown fields when the map is inlined into the parent. Unknown field
    /// rejection is instead enforced per entry on `ServiceDescriptor`.
    #[serde(flatten)]
    pub(crate) service_map: crate::parsed::common::Services,
}

impl SkipSerializing for Services {
    fn skip_serializing(&self) -> bool {
        // Destructuring here prevents us from missing new fields if they're
        // added in the future.
        let Services {
            auto_start,
            service_map,
        } = self;
        auto_start.is_none() && service_map.skip_serializing()
    }
}

impl Services {
    pub fn validate(&self) -> Result<(), ManifestError> {
        self.service_map.validate()
    }

    /// Create a new [Services] instance with services for systems other than
    /// `system` filtered out.
    ///
    /// Clone the services rather than filter in place to avoid accidental
    /// mutation of the original in memory manifest/lockfile. Preserves the
    /// `auto_start` setting.
    pub fn copy_for_system(&self, system: &System) -> Self {
        Services {
            auto_start: self.auto_start,
            service_map: self.service_map.copy_for_system(system),
        }
    }
}

impl Inner for Services {
    type Inner = BTreeMap<String, ServiceDescriptor>;

    fn inner(&self) -> &Self::Inner {
        self.service_map.inner()
    }

    fn inner_mut(&mut self) -> &mut Self::Inner {
        self.service_map.inner_mut()
    }

    fn into_inner(self) -> Self::Inner {
        self.service_map.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::test_helpers::with_latest_schema;

    /// `options.activate.add-sbin = true` round-trips through v1.12.0.
    #[test]
    fn parses_add_sbin_true() {
        let manifest = with_latest_schema(indoc! {r#"
            [options.activate]
            add-sbin = true
        "#});
        let parsed: ManifestV1_12_0 = toml_edit::de::from_str(&manifest).unwrap();
        assert_eq!(parsed.options.activate, ActivateOptions {
            mode: None,
            add_sbin: Some(true),
        });
    }

    /// Omitting `add-sbin` leaves the field as `None`.
    #[test]
    fn add_sbin_defaults_to_none() {
        let manifest = with_latest_schema("");
        let parsed: ManifestV1_12_0 = toml_edit::de::from_str(&manifest).unwrap();
        assert_eq!(parsed.options.activate, ActivateOptions::default());
        assert_eq!(parsed.options.activate.add_sbin, None);
    }

    /// A manifest whose `ActivateOptions` defaults to `None`/`None` must not
    /// emit any keys into the serialized output.
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
