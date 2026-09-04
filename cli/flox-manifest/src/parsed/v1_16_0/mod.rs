use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use flox_core::data::System;
#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::{
    alphanum_string,
    btree_map_strategy,
    optional_btree_map,
    optional_string,
    optional_vec_of_strings,
};
use indoc::formatdoc;
use itertools::Itertools;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use systemd::unit::ServiceUnit;

use crate::interfaces::{AsTypedOnlyManifest, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{Containerize, Include, KnownSchemaVersion, Options, Vars};
use crate::parsed::v1_10_0::{Install, ManifestPackageDescriptor};
pub use crate::parsed::v1_11_0::MinimumCliVersion;
pub use crate::parsed::v1_13_0::{
    Build,
    BuildDescriptor,
    BuildSandbox,
    Profile,
    ProfileDeactivate,
};
pub use crate::parsed::v1_14_0::Plugins;
pub use crate::parsed::v1_15_0::Hook;
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
pub struct ManifestV1_16_0 {
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
    /// Free-form data provided by installed plugins, keyed by plugin
    /// package name (`[plugins.<pkg-name>]`). Each plugin defines and
    /// validates the shape of its own table; Flox does not interpret it.
    /// The convention for secrets plugins is a flat table of
    /// `ENV_VAR_NAME = "path/to/secret/in/store"`, read by the plugin's
    /// `profile.d` script at activation.
    #[serde(default)]
    #[serde(skip_serializing_if = "Plugins::skip_serializing")]
    pub plugins: Plugins,
}
impl_pkg_lookup!(crate::parsed::v1_10_0, ManifestV1_16_0);

// You can't derive `Default` because `schema-version` is a `String`,
// which just defaults to an empty string.
impl Default for ManifestV1_16_0 {
    fn default() -> Self {
        Self {
            schema_version: "1.16.0".into(),
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
            plugins: Default::default(),
        }
    }
}

impl AsTypedOnlyManifest for ManifestV1_16_0 {
    fn as_typed_only(&self) -> crate::Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::V1_16_0(self.clone()),
            },
        }
    }
}

impl SchemaVersion for ManifestV1_16_0 {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        KnownSchemaVersion::V1_16_0
    }
}

/// Service configuration for V1_16_0.
///
/// This is a version-specific copy of `v1_12_0::Services` because V1_16_0's
/// [ServiceDescriptor] adds `depends-on` and `shutdown.{timeout-seconds,signal}`.
/// Unlike v1_12_0 the service map is a plain `BTreeMap` rather than a wrapped
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
            let daemonizes = desc.is_daemon.is_some_and(|is_daemon| is_daemon);
            // A daemon detaches from the process manager, so it can only be
            // stopped with an explicit command — a signal alone is not enough.
            let has_shutdown_cmd = desc
                .shutdown
                .as_ref()
                .is_some_and(|shutdown| shutdown.command.is_some());
            if daemonizes && !has_shutdown_cmd {
                bad_services.push(name.clone());
            }
        }
        let list = bad_services
            .into_iter()
            .map(|name| format!("- {name}"))
            .join("\n");
        if !list.is_empty() {
            let msg = formatdoc! {"
                Services that spawn daemon processes must supply a shutdown command.

                The following services did not specify a shutdown command:
                {list}
            "};
            return Err(ManifestError::InvalidServiceConfig(msg));
        }

        // A shutdown signal must be a valid OS signal number. An out-of-range
        // value is otherwise passed to process-compose unchecked and only
        // silently falls back to the default shutdown behavior.
        for (name, desc) in self.service_map.iter() {
            let Some(shutdown) = desc.shutdown.as_ref() else {
                continue;
            };
            let Some(signal) = shutdown.signal else {
                continue;
            };
            if !(1..=31).contains(&signal) {
                let msg = formatdoc! {"
                    Service '{name}' has an invalid shutdown signal ({signal}).
                    Use a signal number between 1 and 31, for example 15 for SIGTERM or 2 for SIGINT.
                "};
                return Err(ManifestError::InvalidServiceConfig(msg));
            }
            // The process manager runs the shutdown command instead of
            // delivering a signal, so a signal set alongside a command would
            // be silently ignored — dead config that reads as active.
            if shutdown.command.is_some() {
                let msg = formatdoc! {"
                    Service '{name}' sets both a shutdown command and a shutdown signal.
                    Remove 'shutdown.signal': the shutdown command is run instead of delivering a signal.
                "};
                return Err(ManifestError::InvalidServiceConfig(msg));
            }
        }

        Ok(())
    }

    /// Verify that every `depends-on` target names a service in this set.
    ///
    /// A misspelled `depends-on` key is otherwise only reported by
    /// process-compose, in its own vocabulary, when the services are started.
    /// Only the existence of the target is checked; ordering cycles are left to
    /// the process manager.
    ///
    /// This is separate from [`Services::validate`] because it is only
    /// conclusive against a *complete* set of services. A manifest with an
    /// `[include]` section does not have one until its includes are composed
    /// in, so the caller decides when the set it holds is complete enough to
    /// judge a missing target: at parse time for a manifest that includes
    /// nothing, and against the composed manifest when the environment is
    /// built. Rejecting an included target at parse time would tell the author
    /// to correct a spelling that is right.
    ///
    /// Pass the unfiltered services. A target that exists but does not run on
    /// the current system is a different case, handled by
    /// [`Services::drop_depends_on_for_other_systems`].
    pub fn validate_depends_on_targets(&self) -> Result<(), ManifestError> {
        for (name, desc) in self.service_map.iter() {
            let Some(depends_on) = desc.depends_on.as_ref() else {
                continue;
            };
            for target in depends_on.keys() {
                if !self.service_map.contains_key(target) {
                    let msg = formatdoc! {"
                        Service '{name}' depends on service '{target}', which is not defined in this environment.
                        Check the spelling of '{target}', or add a '[services.{target}]' section.
                    "};
                    return Err(ManifestError::InvalidServiceConfig(msg));
                }
            }
        }
        Ok(())
    }

    /// Remove `depends-on` edges whose target is absent from this
    /// (system-filtered) set, and report what was removed.
    ///
    /// Call this on the result of [`Services::copy_for_system`]. `systems`
    /// filtering can drop a service that another service depends on, and
    /// process-compose rejects an edge naming a process its config does not
    /// define.
    ///
    /// The edge is removed rather than rejected because on such a system there
    /// is nothing for the dependent service to wait for: the manifest says the
    /// depended-on service does not run here. Rejecting it would fail the
    /// environment build, and the build is shared by every command that
    /// materializes an environment — `install`, `edit`, `activate`, `upgrade` —
    /// so a service ordering mistake would block all of them on the systems
    /// where the dependency was never meant to run. The caller reports the
    /// removals instead.
    #[must_use = "the returned edges are the only record that any were removed"]
    pub fn drop_depends_on_for_other_systems(&mut self) -> Vec<DroppedDependency> {
        let defined: BTreeSet<String> = self.service_map.keys().cloned().collect();
        let mut dropped = Vec::new();
        for (name, desc) in self.service_map.iter_mut() {
            let Some(depends_on) = desc.depends_on.as_mut() else {
                continue;
            };
            depends_on.retain(|target, _| {
                if defined.contains(target) {
                    return true;
                }
                dropped.push(DroppedDependency {
                    service: name.clone(),
                    target: target.clone(),
                });
                false
            });
            if depends_on.is_empty() {
                desc.depends_on = None;
            }
        }
        dropped
    }

    /// The order in which to start `names`: dependencies before dependents.
    ///
    /// The process manager only defers a starting service on a dependency that
    /// has itself been requested — an edge to a process that was never started
    /// counts as satisfied. Requesting services in dependency order is what
    /// makes the deferral engage, so that `depends-on` holds regardless of how
    /// service names sort.
    ///
    /// Names not defined in this set keep their place, and edges leaving
    /// `names` do not order anything. Should the remaining names ever form a
    /// cycle they keep their input order; the process manager rejects cyclic
    /// configs at load, so that is a fallback, not a supported input.
    pub fn start_order(&self, names: Vec<String>) -> Vec<String> {
        let requested: BTreeSet<String> = names.iter().cloned().collect();
        let mut placed: BTreeSet<String> = BTreeSet::new();
        let mut ordered: Vec<String> = Vec::with_capacity(names.len());
        let mut remaining = names;
        while !remaining.is_empty() {
            let mut blocked = Vec::new();
            let mut progressed = false;
            for name in remaining {
                let deps_placed = self
                    .service_map
                    .get(&name)
                    .and_then(|desc| desc.depends_on.as_ref())
                    .is_none_or(|deps| {
                        deps.keys()
                            .all(|target| !requested.contains(target) || placed.contains(target))
                    });
                if deps_placed {
                    placed.insert(name.clone());
                    ordered.push(name);
                    progressed = true;
                } else {
                    blocked.push(name);
                }
            }
            if !progressed {
                ordered.extend(blocked);
                break;
            }
            remaining = blocked;
        }
        ordered
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
/// V1_16_0 adds `depends-on` and a richer [ServiceShutdown]; it is otherwise
/// identical.
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

    /// Other services that must reach a given state before this service starts.
    ///
    /// Maps the name of a depended-on service to the condition that must be
    /// satisfied before this service is started. Passed straight through to
    /// process-compose's `depends_on`.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_btree_map::<ServiceDependency>(5, 3)")
    )]
    pub depends_on: Option<BTreeMap<String, ServiceDependency>>,

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
/// V1_16_0 makes `command` optional and adds `timeout-seconds` and `signal`.
/// Keeping the new shape out of the common struct is what stops older schema
/// versions (whose `ServiceDescriptor` uses `common::ServiceShutdown`) from
/// accepting it.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ServiceShutdown {
    /// What command to run to shut down the service instead of delivering a
    /// signal. Required when `is-daemon` is `true`.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_string(3)")
    )]
    pub command: Option<String>,
    /// How long to wait, in seconds, for the service to shut down — whether
    /// via a shutdown command or a signal — before it is killed. The process
    /// manager applies its own default of 10 seconds when unset. Zero is not a
    /// valid value because the process manager treats it as unset.
    pub timeout_seconds: Option<NonZeroU32>,
    /// The signal number to send to shut the service down (for example `15`
    /// for `SIGTERM`, `2` for `SIGINT`). Cannot be combined with a shutdown
    /// `command`: the process manager runs the command instead of delivering
    /// a signal, so a signal set alongside one would be silently ignored.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "proptest::option::of(1..32i32)")
    )]
    pub signal: Option<i32>,
}

/// The state a depended-on service must reach before a dependent service is
/// allowed to start.
///
/// Serializes to the literal condition strings that process-compose expects in
/// a `depends_on.<name>.condition` field.
///
/// process-compose also accepts `process_healthy`, which is deliberately not
/// offered here: it only means anything once readiness probes are surfaced in
/// the manifest, so it arrives with them rather than accepting a condition that
/// can never be satisfied.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "snake_case")]
pub enum ServiceStartCondition {
    /// The depended-on service's process has been started.
    ProcessStarted,
    /// The depended-on service's process has exited, with any status.
    ProcessCompleted,
    /// The depended-on service's process has exited successfully (status 0).
    ProcessCompletedSuccessfully,
}

impl ServiceStartCondition {
    /// The literal condition string process-compose expects for this variant.
    pub fn as_process_compose_str(&self) -> &'static str {
        match self {
            ServiceStartCondition::ProcessStarted => "process_started",
            ServiceStartCondition::ProcessCompleted => "process_completed",
            ServiceStartCondition::ProcessCompletedSuccessfully => "process_completed_successfully",
        }
    }
}

/// A `depends-on` edge removed by [`Services::drop_depends_on_for_other_systems`]
/// because the depended-on service does not run on the current system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedDependency {
    /// The service that declared the edge.
    pub service: String,
    /// The depended-on service, which is not defined for the current system.
    pub target: String,
}

/// A single `depends-on` edge for a service: wait for `condition` on the named
/// service before starting.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ServiceDependency {
    /// The state the depended-on service must reach before this service starts.
    pub condition: ServiceStartCondition,
}

// Conversions from the common types, used by the V1_15_0 -> V1_16_0 migration.
// The new fields default to None (and the required `command` becomes
// `Some(command)`), which is what makes the migration lossless.
impl From<crate::parsed::common::ServiceShutdown> for ServiceShutdown {
    fn from(shutdown: crate::parsed::common::ServiceShutdown) -> Self {
        let crate::parsed::common::ServiceShutdown { command } = shutdown;
        ServiceShutdown {
            command: Some(command),
            timeout_seconds: None,
            signal: None,
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
            depends_on: None,
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

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;

    #[test]
    fn start_condition_maps_to_process_compose_strings() {
        assert_eq!(
            ServiceStartCondition::ProcessStarted.as_process_compose_str(),
            "process_started"
        );
        assert_eq!(
            ServiceStartCondition::ProcessCompleted.as_process_compose_str(),
            "process_completed"
        );
        assert_eq!(
            ServiceStartCondition::ProcessCompletedSuccessfully.as_process_compose_str(),
            "process_completed_successfully"
        );
    }

    #[test]
    fn service_descriptor_parses_depends_on_and_shutdown_knobs() {
        let toml = indoc::indoc! {r#"
            command = "run-web"
            depends-on.db = { condition = "process_completed_successfully" }
            shutdown = { command = "stop-web", timeout-seconds = 30, signal = 2 }
        "#};
        let desc: ServiceDescriptor = toml_edit::de::from_str(toml).unwrap();

        let shutdown = desc.shutdown.expect("shutdown should parse");
        assert_eq!(shutdown.command.as_deref(), Some("stop-web"));
        assert_eq!(shutdown.timeout_seconds, NonZeroU32::new(30));
        assert_eq!(shutdown.signal, Some(2));

        let deps = desc.depends_on.expect("depends-on should parse");
        assert_eq!(
            deps["db"].condition,
            ServiceStartCondition::ProcessCompletedSuccessfully
        );
    }

    #[test]
    fn shutdown_may_set_only_a_timeout_without_a_command() {
        // A non-daemon service may bound its shutdown with just a timeout and
        // no explicit command.
        let toml = indoc::indoc! {r#"
            command = "run-web"
            shutdown = { timeout-seconds = 8 }
        "#};
        let desc: ServiceDescriptor = toml_edit::de::from_str(toml).unwrap();
        let shutdown = desc.shutdown.expect("shutdown should parse");
        assert_eq!(shutdown.command, None);
        assert_eq!(shutdown.timeout_seconds, NonZeroU32::new(8));
    }

    #[test]
    fn rejects_zero_shutdown_timeout() {
        // process-compose treats timeout 0 as "unset", so NonZeroU32 rejects it
        // at parse time rather than silently applying the default.
        let toml = indoc::indoc! {r#"
            command = "run-web"
            shutdown = { command = "stop-web", timeout-seconds = 0 }
        "#};
        assert!(toml_edit::de::from_str::<ServiceDescriptor>(toml).is_err());
    }

    #[test]
    fn process_healthy_condition_not_yet_supported() {
        // `process_healthy` depends on readiness probes, which are a later
        // stage; until those land the condition is intentionally rejected.
        let toml = indoc::indoc! {r#"
            command = "run-web"
            depends-on.db = { condition = "process_healthy" }
        "#};
        assert!(toml_edit::de::from_str::<ServiceDescriptor>(toml).is_err());
    }

    fn service(descriptor: ServiceDescriptor) -> Services {
        Services {
            auto_start: None,
            service_map: BTreeMap::from([("svc".to_string(), descriptor)]),
        }
    }

    fn descriptor(command: &str) -> ServiceDescriptor {
        ServiceDescriptor {
            command: command.to_string(),
            vars: None,
            is_daemon: None,
            shutdown: None,
            depends_on: None,
            systemd: None,
            systems: None,
        }
    }

    #[test]
    fn daemon_requires_shutdown_command_not_just_a_timeout() {
        // A daemon whose shutdown table only sets a timeout must still be
        // rejected: the process manager cannot signal the daemon directly and
        // needs an explicit command to stop it.
        let timeout_only = service(ServiceDescriptor {
            is_daemon: Some(true),
            shutdown: Some(ServiceShutdown {
                command: None,
                timeout_seconds: NonZeroU32::new(8),
                signal: None,
            }),
            ..descriptor("run")
        });
        assert!(timeout_only.validate().is_err());

        let with_command = service(ServiceDescriptor {
            is_daemon: Some(true),
            shutdown: Some(ServiceShutdown {
                command: Some("stop".to_string()),
                timeout_seconds: NonZeroU32::new(8),
                signal: None,
            }),
            ..descriptor("run")
        });
        assert!(with_command.validate().is_ok());
    }

    #[test]
    fn rejects_out_of_range_shutdown_signal() {
        let bad_signal = service(ServiceDescriptor {
            shutdown: Some(ServiceShutdown {
                command: None,
                timeout_seconds: None,
                signal: Some(99),
            }),
            ..descriptor("run")
        });
        assert!(bad_signal.validate().is_err());

        let good_signal = service(ServiceDescriptor {
            shutdown: Some(ServiceShutdown {
                command: None,
                timeout_seconds: None,
                signal: Some(15),
            }),
            ..descriptor("run")
        });
        assert!(good_signal.validate().is_ok());

        // Boundaries of the inclusive 1..=31 range: an off-by-one in either
        // direction would otherwise slip past the two mid-range cases above.
        let lower_bound = service(ServiceDescriptor {
            shutdown: Some(ServiceShutdown {
                command: None,
                timeout_seconds: None,
                signal: Some(1),
            }),
            ..descriptor("run")
        });
        assert!(lower_bound.validate().is_ok());

        let upper_bound = service(ServiceDescriptor {
            shutdown: Some(ServiceShutdown {
                command: None,
                timeout_seconds: None,
                signal: Some(31),
            }),
            ..descriptor("run")
        });
        assert!(upper_bound.validate().is_ok());

        // One past each edge of the accepting boundaries above: a widening
        // off-by-one (e.g. `0..=31` or `1..=32`) would slip past the
        // accepting cases alone.
        let below_range = service(ServiceDescriptor {
            shutdown: Some(ServiceShutdown {
                command: None,
                timeout_seconds: None,
                signal: Some(0),
            }),
            ..descriptor("run")
        });
        assert!(below_range.validate().is_err());

        let above_range = service(ServiceDescriptor {
            shutdown: Some(ServiceShutdown {
                command: None,
                timeout_seconds: None,
                signal: Some(32),
            }),
            ..descriptor("run")
        });
        assert!(above_range.validate().is_err());
    }

    #[test]
    fn rejects_signal_combined_with_shutdown_command() {
        // The process manager runs the shutdown command instead of delivering
        // a signal, so accepting both would leave `signal` as dead config.
        let both = service(ServiceDescriptor {
            shutdown: Some(ServiceShutdown {
                command: Some("stop".to_string()),
                timeout_seconds: None,
                signal: Some(15),
            }),
            ..descriptor("run")
        });
        let err = both.validate().unwrap_err();
        assert!(
            err.to_string()
                .contains("both a shutdown command and a shutdown signal"),
            "unexpected error message: {err}",
        );

        // A command alone (with or without a timeout) stays valid.
        let command_only = service(ServiceDescriptor {
            shutdown: Some(ServiceShutdown {
                command: Some("stop".to_string()),
                timeout_seconds: NonZeroU32::new(8),
                signal: None,
            }),
            ..descriptor("run")
        });
        assert!(command_only.validate().is_ok());
    }

    #[test]
    fn rejects_depends_on_target_that_names_no_service() {
        // A misspelled dependency name is caught when the manifest is
        // validated, rather than by process-compose at 'flox services start'.
        let web = ServiceDescriptor {
            depends_on: Some(BTreeMap::from([(
                "migratons".to_string(),
                ServiceDependency {
                    condition: ServiceStartCondition::ProcessCompletedSuccessfully,
                },
            )])),
            ..descriptor("run-web")
        };
        let services = Services {
            auto_start: None,
            service_map: BTreeMap::from([
                ("migrations".to_string(), descriptor("run-migrations")),
                ("web".to_string(), web),
            ]),
        };

        let err = services.validate_depends_on_targets().unwrap_err();
        assert!(
            err.to_string().contains("migratons"),
            "error should name the misspelled dependency, got: {err}"
        );
    }

    /// `web` runs everywhere and depends on `db`, which only runs on
    /// x86_64-linux.
    fn cross_system_services() -> Services {
        let db = ServiceDescriptor {
            systems: Some(vec!["x86_64-linux".to_string()]),
            ..descriptor("run-db")
        };
        let web = ServiceDescriptor {
            depends_on: Some(BTreeMap::from([("db".to_string(), ServiceDependency {
                condition: ServiceStartCondition::ProcessStarted,
            })])),
            ..descriptor("run-web")
        };
        Services {
            auto_start: None,
            service_map: BTreeMap::from([("db".to_string(), db), ("web".to_string(), web)]),
        }
    }

    #[test]
    fn start_order_places_dependencies_before_dependents() {
        // `app` sorts before `db`, so an alphabetical start order would
        // request the dependent first — which the process manager reads as
        // "no dependency".
        let app = ServiceDescriptor {
            depends_on: Some(BTreeMap::from([("db".to_string(), ServiceDependency {
                condition: ServiceStartCondition::ProcessStarted,
            })])),
            ..descriptor("run-app")
        };
        let services = Services {
            auto_start: None,
            service_map: BTreeMap::from([
                ("app".to_string(), app),
                ("db".to_string(), descriptor("run-db")),
            ]),
        };

        assert_eq!(
            services.start_order(vec!["app".to_string(), "db".to_string()]),
            vec!["db".to_string(), "app".to_string()],
        );
    }

    #[test]
    fn start_order_orders_chains() {
        // a -> b -> c by dependency, requested in alphabetical order.
        let a = ServiceDescriptor {
            depends_on: Some(BTreeMap::from([("b".to_string(), ServiceDependency {
                condition: ServiceStartCondition::ProcessCompletedSuccessfully,
            })])),
            ..descriptor("run-a")
        };
        let b = ServiceDescriptor {
            depends_on: Some(BTreeMap::from([("c".to_string(), ServiceDependency {
                condition: ServiceStartCondition::ProcessStarted,
            })])),
            ..descriptor("run-b")
        };
        let services = Services {
            auto_start: None,
            service_map: BTreeMap::from([
                ("a".to_string(), a),
                ("b".to_string(), b),
                ("c".to_string(), descriptor("run-c")),
            ]),
        };

        assert_eq!(
            services.start_order(vec!["a".to_string(), "b".to_string(), "c".to_string()]),
            vec!["c".to_string(), "b".to_string(), "a".to_string()],
        );
    }

    #[test]
    fn start_order_ignores_dependencies_outside_the_requested_set() {
        // Only `app` is requested, so its edge to `db` orders nothing and the
        // input comes back unchanged.
        let app = ServiceDescriptor {
            depends_on: Some(BTreeMap::from([("db".to_string(), ServiceDependency {
                condition: ServiceStartCondition::ProcessStarted,
            })])),
            ..descriptor("run-app")
        };
        let services = Services {
            auto_start: None,
            service_map: BTreeMap::from([
                ("app".to_string(), app),
                ("db".to_string(), descriptor("run-db")),
            ]),
        };

        assert_eq!(services.start_order(vec!["app".to_string()]), vec![
            "app".to_string()
        ]);
    }

    #[test]
    fn start_order_keeps_input_order_on_a_cycle() {
        // The process manager rejects cyclic configs at load, so this only
        // asserts the fallback: every requested name is still returned.
        let a = ServiceDescriptor {
            depends_on: Some(BTreeMap::from([("b".to_string(), ServiceDependency {
                condition: ServiceStartCondition::ProcessStarted,
            })])),
            ..descriptor("run-a")
        };
        let b = ServiceDescriptor {
            depends_on: Some(BTreeMap::from([("a".to_string(), ServiceDependency {
                condition: ServiceStartCondition::ProcessStarted,
            })])),
            ..descriptor("run-b")
        };
        let services = Services {
            auto_start: None,
            service_map: BTreeMap::from([("a".to_string(), a), ("b".to_string(), b)]),
        };

        assert_eq!(
            services.start_order(vec!["a".to_string(), "b".to_string()]),
            vec!["a".to_string(), "b".to_string()],
        );
    }

    #[test]
    fn keeps_depends_on_edge_where_both_services_run() {
        let linux = "x86_64-linux".to_string();
        let mut for_linux = cross_system_services().copy_for_system(&linux);

        assert_eq!(for_linux.drop_depends_on_for_other_systems(), vec![]);
        assert_eq!(for_linux, cross_system_services().copy_for_system(&linux));
    }

    #[test]
    fn drops_depends_on_edge_to_service_filtered_out_on_this_system() {
        // On aarch64-darwin the filtered set has no `db`, so `web`'s edge has
        // nothing to wait for and is removed rather than failing the build.
        let mut for_darwin = cross_system_services().copy_for_system(&"aarch64-darwin".to_string());

        assert_eq!(for_darwin.drop_depends_on_for_other_systems(), vec![
            DroppedDependency {
                service: "web".to_string(),
                target: "db".to_string(),
            }
        ]);
        // The whole table goes, not just the edge, so the generated config
        // carries no empty `depends_on` for process-compose to interpret.
        assert_eq!(for_darwin, Services {
            auto_start: None,
            service_map: BTreeMap::from([("web".to_string(), descriptor("run-web"))]),
        });
    }
}
