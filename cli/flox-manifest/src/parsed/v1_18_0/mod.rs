use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::PathBuf;

use flox_core::activate::mode::ActivateMode;
use flox_core::data::System;
use flox_core::data::environment_ref::RemoteEnvironmentRef;
#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::{optional_string, optional_vec_of_strings};
use indoc::formatdoc;
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::interfaces::{AsTypedOnlyManifest, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{Allows, Containerize, KnownSchemaVersion, SemverOptions, Vars};
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
// Service shape is unchanged since v1_16_0.
pub use crate::parsed::v1_16_0::{
    DroppedDependency,
    ServiceDependency,
    ServiceDescriptor,
    ServiceShutdown,
    ServiceStartCondition,
    Services,
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
pub struct ManifestV1_18_0 {
    /// Which schema version this manifest adheres to.
    ///
    /// Must be a valid Flox CLI version listed in [`KnownSchemaVersion`].
    #[serde(rename = "schema-version")]
    pub schema_version: String,
    /// A human-readable description of the environment's purpose.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_string(5)")
    )]
    pub description: Option<String>,
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
impl_pkg_lookup!(crate::parsed::v1_10_0, ManifestV1_18_0);

// You can't derive `Default` because `schema-version` is a `String`,
// which just defaults to an empty string.
impl Default for ManifestV1_18_0 {
    fn default() -> Self {
        Self {
            schema_version: "1.18.0".into(),
            description: Default::default(),
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

impl AsTypedOnlyManifest for ManifestV1_18_0 {
    fn as_typed_only(&self) -> crate::Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::V1_18_0(self.clone()),
            },
        }
    }
}

impl SchemaVersion for ManifestV1_18_0 {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        KnownSchemaVersion::V1_18_0
    }
}

/// Manifest options for V1_18_0: identical to `common::Options` except that
/// `activate` is the V1_18_0 [`ActivateOptions`]. Earlier schema versions keep
/// using `common::Options`.
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

/// Activation options for V1_18_0: adds `upgrade-notifications`.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ActivateOptions {
    pub mode: Option<ActivateMode>,
    /// Whether `flox activate` notifies about available upgrades for this
    /// environment. Setting this to `false` suppresses the notification for
    /// everyone who activates the environment; it can't re-enable
    /// notifications that a user disabled with the `upgrade_notifications`
    /// config key.
    pub upgrade_notifications: Option<bool>,
}

impl SkipSerializing for ActivateOptions {
    /// Don't write a struct of None's into the lockfile but also don't
    /// explicitly check fields which we might forget to update.
    fn skip_serializing(&self) -> bool {
        self == &ActivateOptions::default()
    }
}

// Conversion from the common type, used by the V1_17_0 -> V1_18_0 migration.
// The new `upgrade_notifications` field defaults to None, which is what makes
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
            activate: ActivateOptions {
                mode: activate.mode,
                upgrade_notifications: None,
            },
        }
    }
}

/// The section where users can declare dependencies on other environments.
///
/// From V1_18_0 on, include descriptors can set `auto-upgrade`.
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

impl Include {
    /// Check for settings that contradict each other, which the types allow
    /// because the errors of untagged enum variants can't be reported
    pub fn validate(&self) -> Result<(), ManifestError> {
        for include in &self.environments {
            if let IncludeDescriptor::Remote {
                generation: Some(generation),
                auto_upgrade: Some(true),
                ..
            } = include
            {
                return Err(ManifestError::InvalidIncludeConfig(formatdoc! {"
                    Included environment '{include}' sets both 'generation = {generation}' and 'auto-upgrade = true'.
                    Remove 'generation' to use the latest generation, or remove 'auto-upgrade' to stay on generation {generation}."
                }));
            }
        }
        Ok(())
    }
}

/// The structure for how a user is able to declare a dependency on an environment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
#[serde(
    untagged,
    expecting = "expected { dir = <dir>, [name = <name>], [auto-upgrade = <bool>] } OR { remote = <owner/name>, [name = <name>], [generation = <generation>], [auto-upgrade = <bool>] }"
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
        /// Whether commands use the latest changes that the included
        /// environment has locked, without 'flox include upgrade'.
        /// Defaults to true if the directory holds a path environment.
        #[serde(
            rename = "auto-upgrade",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        auto_upgrade: Option<bool>,
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
        /// Whether commands use the latest generation of the environment,
        /// without 'flox include upgrade'.
        /// Defaults to false, and can't be true together with `generation`.
        #[serde(
            rename = "auto-upgrade",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        auto_upgrade: Option<bool>,
    },
}

/// Whether an included environment's latest changes are used without
/// 'flox include upgrade'
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoUpgrade {
    Always,
    Never,
    /// If the included directory holds a path environment, rather than a
    /// managed one
    IfPathEnvironment,
}

impl IncludeDescriptor {
    /// Whether `other` includes the same environment under the same name,
    /// regardless of whether either upgrades it automatically
    pub fn includes_same_environment(&self, other: &IncludeDescriptor) -> bool {
        self.without_auto_upgrade() == other.without_auto_upgrade()
    }

    fn without_auto_upgrade(&self) -> IncludeDescriptor {
        let mut descriptor = self.clone();
        match &mut descriptor {
            IncludeDescriptor::Local { auto_upgrade, .. }
            | IncludeDescriptor::Remote { auto_upgrade, .. } => *auto_upgrade = None,
        }
        descriptor
    }

    /// A remote environment pinned to a generation never changes,
    /// so it's never upgraded automatically.
    pub fn auto_upgrade(&self) -> AutoUpgrade {
        match self {
            IncludeDescriptor::Local {
                auto_upgrade: Some(true),
                ..
            }
            | IncludeDescriptor::Remote {
                generation: None,
                auto_upgrade: Some(true),
                ..
            } => AutoUpgrade::Always,
            IncludeDescriptor::Local {
                auto_upgrade: None, ..
            } => AutoUpgrade::IfPathEnvironment,
            IncludeDescriptor::Local {
                auto_upgrade: Some(false),
                ..
            }
            | IncludeDescriptor::Remote { .. } => AutoUpgrade::Never,
        }
    }
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

// Conversions from the common types, used by the V1_17_0 -> V1_18_0 migration.
// `auto-upgrade` stays unset.
impl From<crate::parsed::common::Include> for Include {
    fn from(include: crate::parsed::common::Include) -> Self {
        Include {
            environments: include.environments.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<crate::parsed::common::IncludeDescriptor> for IncludeDescriptor {
    fn from(descriptor: crate::parsed::common::IncludeDescriptor) -> Self {
        match descriptor {
            crate::parsed::common::IncludeDescriptor::Local { dir, name } => {
                IncludeDescriptor::Local {
                    dir,
                    name,
                    auto_upgrade: None,
                }
            },
            crate::parsed::common::IncludeDescriptor::Remote {
                remote,
                name,
                generation,
            } => IncludeDescriptor::Remote {
                remote,
                name,
                generation,
                auto_upgrade: None,
            },
        }
    }
}
