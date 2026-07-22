use std::collections::BTreeMap;
use std::num::NonZeroU32;

use flox_core::data::System;
#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::{alphanum_string, btree_map_strategy, optional_vec_of_strings};
use indoc::formatdoc;
use itertools::Itertools;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use systemd::unit::ServiceUnit;

use crate::interfaces::{AsTypedOnlyManifest, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{Containerize, Hook, Include, KnownSchemaVersion, Options, Vars};
use crate::parsed::v1_10_0::{Install, ManifestPackageDescriptor};
pub use crate::parsed::v1_11_0::MinimumCliVersion;
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

/// Service configuration for V1_14_0.
///
/// This is a version-specific copy of `v1_12_0::Services` because V1_14_0's
/// [ServiceDescriptor] adds `shutdown.timeout-seconds`. Unlike v1_12_0 the
/// service map is a plain `BTreeMap` rather than a wrapped
/// `common::Services`: the uniform `CommonFields::services()` accessor that
/// motivated the wrapping was replaced by per-version validation dispatch.
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
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "btree_map_strategy::<ServiceDescriptor>(5, 3)")
    )]
    pub(crate) service_map: BTreeMap<String, ServiceDescriptor>,
}

impl SkipSerializing for Services {
    fn skip_serializing(&self) -> bool {
        // Destructuring here prevents us from missing new fields if they're
        // added in the future.
        let Services {
            auto_start,
            service_map,
        } = self;
        auto_start.is_none() && service_map.is_empty()
    }
}

impl Services {
    pub fn validate(&self) -> Result<(), ManifestError> {
        let mut bad_services = vec![];
        for (name, desc) in self.service_map.iter() {
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
            Err(ManifestError::InvalidServiceConfig(msg))
        }
    }

    /// Create a new [Services] instance with services for systems other than
    /// `system` filtered out.
    ///
    /// Clone the services rather than filter in place to avoid accidental
    /// mutation of the original in memory manifest/lockfile. Preserves the
    /// `auto_start` setting.
    pub fn copy_for_system(&self, system: &System) -> Self {
        let mut service_map = BTreeMap::new();
        for (name, desc) in self.service_map.iter() {
            if desc
                .systems
                .as_ref()
                .is_none_or(|systems| systems.contains(system))
            {
                service_map.insert(name.clone(), desc.clone());
            }
        }
        Services {
            auto_start: self.auto_start,
            service_map,
        }
    }
}

impl Inner for Services {
    type Inner = BTreeMap<String, ServiceDescriptor>;

    fn inner(&self) -> &Self::Inner {
        &self.service_map
    }

    fn inner_mut(&mut self) -> &mut Self::Inner {
        &mut self.service_map
    }

    fn into_inner(self) -> Self::Inner {
        self.service_map
    }
}

/// The definition of a service in a manifest.
///
/// This is a version-specific copy of `common::ServiceDescriptor` because
/// V1_14_0's [ServiceShutdown] adds the `timeout-seconds` field; it is
/// otherwise identical.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ServiceDescriptor {
    /// The command to run to start the service
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "alphanum_string(3)")
    )]
    pub command: String,
    /// Service-specific environment variables
    pub vars: Option<Vars>,
    /// Whether the service spawns a background process (daemon)
    pub is_daemon: Option<bool>,
    /// How to shut down the service
    pub shutdown: Option<ServiceShutdown>,

    /// Additional manual config of the systemd service generated for persistent services
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(
            strategy = "crate::parsed::common::test_helpers::service_unit_with_none_fields()"
        )
    )]
    pub systemd: Option<ServiceUnit>,

    /// Systems to allow running the service on
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub systems: Option<Vec<System>>,
}

/// How to shut down a service.
///
/// This is a version-specific copy of `common::ServiceShutdown` because
/// V1_14_0 adds the `timeout-seconds` field. Keeping the new field out of the
/// common struct is what stops older schema versions (whose
/// `ServiceDescriptor` uses `common::ServiceShutdown`) from accepting it.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ServiceShutdown {
    /// What command to run to shut down the service
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "alphanum_string(3)")
    )]
    pub command: String,
    /// How long to wait, in seconds, for the shutdown command to complete
    /// before the service is killed. The process manager applies its own
    /// default of 10 seconds when unset. Zero is not a valid value because
    /// the process manager treats it as unset.
    pub timeout_seconds: Option<NonZeroU32>,
}

// Conversions from the common types, used by the V1_13_0 -> V1_14_0
// migration. The new `timeout_seconds` field defaults to None, which is what
// makes the migration lossless.
impl From<crate::parsed::common::ServiceShutdown> for ServiceShutdown {
    fn from(shutdown: crate::parsed::common::ServiceShutdown) -> Self {
        let crate::parsed::common::ServiceShutdown { command } = shutdown;
        ServiceShutdown {
            command,
            timeout_seconds: None,
        }
    }
}

impl From<crate::parsed::common::ServiceDescriptor> for ServiceDescriptor {
    fn from(descriptor: crate::parsed::common::ServiceDescriptor) -> Self {
        let crate::parsed::common::ServiceDescriptor {
            command,
            vars,
            is_daemon,
            shutdown,
            systemd,
            systems,
        } = descriptor;
        ServiceDescriptor {
            command,
            vars,
            is_daemon,
            shutdown: shutdown.map(Into::into),
            systemd,
            systems,
        }
    }
}

impl From<crate::parsed::v1_12_0::Services> for Services {
    fn from(services: crate::parsed::v1_12_0::Services) -> Self {
        let auto_start = services.auto_start;
        let service_map = services
            .into_inner()
            .into_iter()
            .map(|(name, descriptor)| (name, descriptor.into()))
            .collect();
        Services {
            auto_start,
            service_map,
        }
    }
}
