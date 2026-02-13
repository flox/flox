use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;

use flox_core::data::System;
use flox_core::data::environment_ref::RemoteEnvironmentRef;
#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::{
    alphanum_and_whitespace_string,
    alphanum_string,
    btree_map_strategy,
    optional_btree_map,
    optional_btree_set,
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

use crate::ManifestError;
use crate::parsed::{SkipSerializing, impl_into_inner};

pub const DEFAULT_GROUP_NAME: &str = "toplevel";
pub const DEFAULT_PRIORITY: u64 = 5;
pub const FILENAME: &str = "manifest.toml";

/// A type holding the different identifiers we've used to represent the schema
/// version of a manifest.
///
/// This is used when we're trying to identify the "shape" of the manifest
/// while handling its untyped form.
#[derive(Debug, Clone)]
pub(crate) enum VersionKind {
    /// A `version = 1` manifest.
    Version(u8),
    /// A `schema-version = "1.10.0"` or later manifest
    SchemaVersion(String),
}

/// All known and valid schema versions supported by the CLI.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub enum KnownSchemaVersion {
    // NOTE: The order in which the enum variants are listed defines
    //       the sort order. Don't mess that up!
    V1,
    V1_10_0,
}

impl KnownSchemaVersion {
    /// Returns the latest schema version.
    pub fn latest() -> Self {
        KnownSchemaVersion::V1_10_0
    }

    /// Returns the oldest supported schema version.
    pub fn oldest() -> Self {
        KnownSchemaVersion::V1
    }

    /// Returns an iterator over all schema versions.
    pub fn iter() -> impl Iterator<Item = KnownSchemaVersion> {
        [KnownSchemaVersion::V1, KnownSchemaVersion::V1_10_0].into_iter()
    }
}

impl TryFrom<VersionKind> for KnownSchemaVersion {
    type Error = ManifestError;

    fn try_from(value: VersionKind) -> Result<Self, Self::Error> {
        match value {
            VersionKind::Version(1) => Ok(KnownSchemaVersion::V1),
            VersionKind::Version(v) => Err(ManifestError::InvalidSchemaVersion(format!("{v}"))),
            VersionKind::SchemaVersion(v) => match v.as_str() {
                "1.10.0" => Ok(KnownSchemaVersion::V1_10_0),
                _ => Err(ManifestError::InvalidSchemaVersion(v.to_string())),
            },
        }
    }
}

impl std::fmt::Display for KnownSchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KnownSchemaVersion::V1 => write!(f, "1"),
            KnownSchemaVersion::V1_10_0 => write!(f, "1.10.0"),
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct PackageDescriptorStorePath {
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "alphanum_string(5)")
    )]
    pub store_path: String,
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub systems: Option<Vec<System>>,
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "proptest::option::of(0..10u64)")
    )]
    pub priority: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct Vars(
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "btree_map_strategy::<String>(5, 3)")
    )]
    pub(crate) BTreeMap<String, String>,
);

impl Vars {
    pub fn from_map(map: BTreeMap<String, String>) -> Self {
        Self(map)
    }
}

impl SkipSerializing for Vars {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

impl_into_inner!(Vars, BTreeMap<String, String>);
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct Hook {
    /// A script that is run at activation time,
    /// in a flox provided bash shell
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "proptest::option::of(alphanum_and_whitespace_string(5))")
    )]
    pub(crate) on_activate: Option<String>,
}

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
}

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
    /// Options that control the behavior of activations.
    #[serde(default)]
    #[serde(skip_serializing_if = "ActivateOptions::skip_serializing")]
    pub activate: ActivateOptions,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct Allows {
    /// Whether to allow packages that are marked as `unfree`
    pub unfree: Option<bool>,
    /// Whether to allow packages that are marked as `broken`
    pub broken: Option<bool>,
    /// A list of license descriptors that are allowed
    #[serde(default)]
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub licenses: Option<Vec<String>>,
}

impl SkipSerializing for Allows {
    fn skip_serializing(&self) -> bool {
        // Destructuring here prevents us from missing new fields if they're
        // added in the future.
        let Allows {
            unfree,
            broken,
            licenses,
        } = self;
        unfree.is_none() && broken.is_none() && licenses.is_none()
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct SemverOptions {
    /// Whether to allow pre-release versions when resolving
    #[serde(default)]
    pub allow_pre_releases: Option<bool>,
}

impl SkipSerializing for SemverOptions {
    fn skip_serializing(&self) -> bool {
        // Destructuring here prevents us from missing new fields if they're
        // added in the future.
        let SemverOptions { allow_pre_releases } = self;
        allow_pre_releases.is_none()
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ActivateOptions {
    pub mode: Option<ActivateMode>,
}

impl SkipSerializing for ActivateOptions {
    /// Don't write a struct of None's into the lockfile but also don't
    /// explicitly check fields which we might forget to update.
    fn skip_serializing(&self) -> bool {
        self == &ActivateOptions::default()
    }
}

#[derive(
    Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq, Ord, PartialOrd, Default, JsonSchema,
)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub enum ActivateMode {
    #[default]
    Dev,
    Run,
}

impl Display for ActivateMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActivateMode::Dev => write!(f, "dev"),
            ActivateMode::Run => write!(f, "run"),
        }
    }
}

impl FromStr for ActivateMode {
    type Err = ManifestError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "dev" => Ok(ActivateMode::Dev),
            "run" => Ok(ActivateMode::Run),
            _ => Err(ManifestError::ActivateModeInvalid),
        }
    }
}

/// A map of service names to service definitions
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct Services(
    #[cfg_attr(
        test,
        proptest(strategy = "btree_map_strategy::<ServiceDescriptor>(5, 3)")
    )]
    pub(crate) BTreeMap<String, ServiceDescriptor>,
);

impl SkipSerializing for Services {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

impl_into_inner!(Services, BTreeMap<String, ServiceDescriptor>);

/// The definition of a service in a manifest
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
    // TODO: This option _requires_ the shutdown command, so we'll need to add
    //       that explanation to the manifest.toml docs and service mgmt guide
    pub is_daemon: Option<bool>,
    /// How to shut down the service
    pub shutdown: Option<ServiceShutdown>,

    /// Additional manual config of the systemd service generated for persistent services
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "test_helpers::service_unit_with_none_fields()")
    )]
    pub systemd: Option<ServiceUnit>,

    /// Systems to allow running the service on
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub systems: Option<Vec<System>>,
}

impl Services {
    pub fn validate(&self) -> Result<(), ManifestError> {
        let mut bad_services = vec![];
        for (name, desc) in self.0.iter() {
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

    /// Create a new [ManifestServices] instance with services
    /// for systems other than `system` filtered out.
    ///
    /// Clone the services rather than filter in place
    /// to avoid accidental mutation of the original in memory manifest/lockfile.
    pub fn copy_for_system(&self, system: &System) -> Self {
        let mut services = BTreeMap::new();
        for (name, desc) in self.0.iter() {
            if desc
                .systems
                .as_ref()
                .is_none_or(|systems| systems.contains(system))
            {
                services.insert(name.clone(), desc.clone());
            }
        }
        Services(services)
    }
}

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
}

/// A map of package ids to package build descriptors
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct Build(
    #[cfg_attr(
        test,
        proptest(strategy = "btree_map_strategy::<BuildDescriptor>(5, 3)")
    )]
    pub(crate) BTreeMap<String, BuildDescriptor>,
);

impl_into_inner!(Build, BTreeMap<String,BuildDescriptor>);

impl SkipSerializing for Build {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

/// The definition of a package built from within the environment
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
    /// Packages from the 'toplevel' group to include in the closure of the build result.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub runtime_packages: Option<Vec<String>>,
    /// Sandbox mode for the build.
    pub sandbox: Option<BuildSandbox>,
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

/// The definition of a package built from within the environment
#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, derive_more::Display, JsonSchema,
)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub enum BuildSandbox {
    Off,
    Pure,
}

/// The definition of a package built from within the environment
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case", untagged)]
pub enum BuildVersion {
    Pure(
        #[cfg_attr(
            any(test, feature = "tests"),
            proptest(strategy = "alphanum_string(3)")
        )]
        String,
    ),
    File {
        file: PathBuf,
    },
    Command {
        #[cfg_attr(
            any(test, feature = "tests"),
            proptest(strategy = "alphanum_string(3)")
        )]
        command: String,
    },
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct Containerize {
    pub config: Option<ContainerizeConfig>,
}

/// Container config derived from
/// https://github.com/opencontainers/image-spec/blob/main/config.md
///
/// Env and Entrypoint are left out since they interfere with our activation implementation
/// Deprecated and reserved keys are also left out
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct ContainerizeConfig {
    /// The username or UID which is a platform-specific structure that allows specific control over which user the process run as.
    /// This acts as a default value to use when the value is not specified when creating a container.
    /// For Linux based systems, all of the following are valid: `user`, `uid`, `user:group`, `uid:gid`, `uid:group`, `user:gid`.
    /// If `group`/`gid` is not specified, the default group and supplementary groups of the given `user`/`uid` in `/etc/passwd` and `/etc/group` from the container are applied.
    /// If `group`/`gid` is specified, supplementary groups from the container are ignored.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_string(3)")
    )]
    pub user: Option<String>,
    /// A set of ports to expose from a container running this image.
    /// Its keys can be in the format of:
    /// `port/tcp`, `port/udp`, `port` with the default protocol being `tcp` if not specified.
    /// These values act as defaults and are merged with any specified when creating a container.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_btree_set(3, 4)")
    )]
    pub exposed_ports: Option<BTreeSet<String>>,
    /// Default arguments to the entrypoint of the container.
    /// These values act as defaults and may be replaced by any specified when creating a container.
    /// Flox sets an entrypoint to activate the containerized environment,
    /// and `cmd` is then run inside the activation, similar to
    /// `flox activate -- cmd`.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_vec_of_strings(3, 4)")
    )]
    pub cmd: Option<Vec<String>>,
    /// A set of directories describing where the process is
    /// likely to write data specific to a container instance.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_btree_set(3, 4)")
    )]
    pub volumes: Option<BTreeSet<String>>,
    /// Sets the current working directory of the entrypoint process in the container.
    /// This value acts as a default and may be replaced by a working directory specified when creating a container.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_string(3)")
    )]
    pub working_dir: Option<String>,
    /// This field contains arbitrary metadata for the container.
    /// This property MUST use the [annotation rules](https://github.com/opencontainers/image-spec/blob/main/annotations.md#rules).
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_btree_map::<String>(3, 4)")
    )]
    pub labels: Option<BTreeMap<String, String>>,
    /// This field contains the system call signal that will be sent to the container to exit. The signal can be a signal name in the format `SIGNAME`, for instance `SIGKILL` or `SIGRTMIN+3`.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_string(3)")
    )]
    pub stop_signal: Option<String>,
}

/// The section where users can declare dependencies on other environments.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct Include {
    #[serde(default)]
    pub environments: Vec<IncludeDescriptor>,
}

impl SkipSerializing for Include {
    fn skip_serializing(&self) -> bool {
        self.environments.is_empty()
    }
}

/// The structure for how a user is able to declare a dependency on an environment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
#[serde(
    untagged,
    expecting = "expected { dir = <dir>, [name = <name>] } OR { remote = <owner/name>, [name = <name>] }"
)]
pub enum IncludeDescriptor {
    Local {
        /// The directory where the environment is located.
        dir: PathBuf,
        /// A name similar to an install ID that a user could use to specify
        /// the environment on the command line e.g. for upgrades, or in an
        /// error message.
        #[cfg_attr(
            any(test, feature = "tests"),
            proptest(strategy = "optional_string(5)")
        )]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Remote {
        /// The remote environment reference in the form `owner/name`.
        #[serde(alias = "reference")]
        remote: RemoteEnvironmentRef,
        /// A name similar to an install ID that a user could use to specify
        /// the environment on the command line e.g. for upgrades, or in an
        /// error message.
        #[cfg_attr(
            any(test, feature = "tests"),
            proptest(strategy = "optional_string(5)")
        )]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(
            any(test, feature = "tests"),
            proptest(strategy = "proptest::option::of(0..10usize)")
        )]
        generation: Option<usize>,
    },
}

impl Display for IncludeDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IncludeDescriptor::Local { dir, name, .. } => {
                write!(f, "{}", name.as_deref().unwrap_or(&dir.to_string_lossy()))
            },
            IncludeDescriptor::Remote { remote, name, .. } => {
                write!(f, "{}", name.as_deref().unwrap_or(&remote.to_string()))
            },
        }
    }
}

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use super::*;

    /// Generate a single ServiceUnit with just enough fields to test `skip_serializing_none`
    /// Generating more than 1(!) value with proptest,
    /// increases the runtime of `proptest!`s to the point that we exhausted our stack space in CI
    pub(super) fn service_unit_with_none_fields() -> impl Strategy<Value = Option<ServiceUnit>> {
        Just(Some(ServiceUnit {
            unit: Some(systemd::unit::Unit {
                ..Default::default()
            }),
            service: Some(systemd::unit::Service {
                ..Default::default()
            }),
        }))
    }
}
