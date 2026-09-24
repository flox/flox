use std::collections::BTreeMap;

#[cfg(any(test, feature = "tests"))]
use flox_test_utils::proptest::{btree_map_strategy, optional_string};
#[cfg(any(test, feature = "tests"))]
use proptest::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::interfaces::{AsTypedOnlyManifest, SchemaVersion, impl_pkg_lookup};
use crate::parsed::common::{
    Containerize,
    DEFAULT_GROUP_NAME,
    Include,
    KnownSchemaVersion,
    Options,
    Vars,
};
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
pub use crate::parsed::v1_16_0::{
    DroppedDependency,
    ServiceDependency,
    ServiceDescriptor,
    ServiceShutdown,
    ServiceStartCondition,
    Services,
};
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
    /// Settings shared by every package in a package group, keyed by
    /// group name.
    #[serde(default)]
    #[serde(rename = "pkg-groups")]
    #[serde(skip_serializing_if = "PkgGroups::skip_serializing")]
    pub pkg_groups: PkgGroups,
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
            pkg_groups: Default::default(),
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

impl ManifestV1_18_0 {
    /// The catalog stability that the packages in `group` resolve against,
    /// or `None` to let the catalog pick its default.
    ///
    /// `group` is the name the packages are locked under, so the default
    /// group is [`DEFAULT_GROUP_NAME`].
    pub fn group_stability(&self, group: &str) -> Option<&str> {
        self.pkg_groups
            .inner()
            .get(group)
            .and_then(|settings| settings.stability.as_deref())
    }

    /// Whether any catalog package is installed into `group`.
    pub fn group_has_packages(&self, group: &str) -> bool {
        self.install.inner().values().any(|descriptor| {
            let ManifestPackageDescriptor::Catalog(catalog) = descriptor else {
                return false;
            };
            catalog.pkg_group.as_deref().unwrap_or(DEFAULT_GROUP_NAME) == group
        })
    }
}

/// Settings for package groups, keyed by the group name that packages
/// reference with `pkg-group`.
///
/// Every package in a group is resolved together against a single catalog
/// page, so settings that constrain resolution belong to the group rather
/// than to individual packages.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct PkgGroups(
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "btree_map_strategy::<PkgGroup>(5, 3)")
    )]
    pub(crate) BTreeMap<String, PkgGroup>,
);

impl_into_inner!(PkgGroups, BTreeMap<String, PkgGroup>);

impl PkgGroups {
    /// The TOML table header for the settings of `group`, with the name
    /// quoted if it isn't a bare key, e.g. `[pkg-groups.legacy]` or
    /// `[pkg-groups."v1.2"]`.
    pub fn table_header(group: &str) -> String {
        format!("[pkg-groups.{}]", toml_edit::Key::new(group))
    }
}

impl SkipSerializing for PkgGroups {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, JsonSchema)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct PkgGroup {
    /// The catalog stability to resolve the group's packages against,
    /// e.g. `stable` or `staging`.
    ///
    /// The catalog validates the value; when unset, the catalog resolves
    /// against its newest page carrying any stability.
    #[cfg_attr(
        any(test, feature = "tests"),
        proptest(strategy = "optional_string(5)")
    )]
    pub stability: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pkg-group names that aren't bare TOML keys are quoted, so that a
    /// name with a dot doesn't read as a nested table.
    #[test]
    fn pkg_groups_table_header_quotes_names() {
        assert_eq!(PkgGroups::table_header("legacy"), "[pkg-groups.legacy]");
        assert_eq!(PkgGroups::table_header("v1.2"), r#"[pkg-groups."v1.2"]"#);
        assert_eq!(
            PkgGroups::table_header("my group"),
            r#"[pkg-groups."my group"]"#
        );
    }
}
