use std::collections::BTreeMap;

use flox_core::data::System;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

pub use crate::parsed::v1_11_0::MinimumCliVersion;
use crate::interfaces::{AsTypedOnlyManifest, CommonFields, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{
    Build,
    Containerize,
    Hook,
    Include,
    KnownSchemaVersion,
    Options,
    Profile,
    ServiceDescriptor,
    Vars,
};
use crate::parsed::v1_10_0::{Install, ManifestPackageDescriptor};
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

    fn options_mut(&mut self) -> &mut super::common::Options {
        &mut self.options
    }

    fn services_auto_start(&self) -> bool {
        self.services.auto_start == Some(true)
    }
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
