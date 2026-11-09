use std::collections::BTreeMap;

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
    BuildVersion,
    Containerize,
    DEFAULT_GROUP_NAME,
    Include,
    KnownSchemaVersion,
    Vars,
};
use crate::parsed::v1_10_0::{Install, ManifestPackageDescriptor};
pub use crate::parsed::v1_11_0::MinimumCliVersion;
pub use crate::parsed::v1_13_0::{BuildSandbox, Profile, ProfileDeactivate};
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
pub use crate::parsed::v1_18_0::ActivateOptions;
pub use crate::parsed::v1_19_0::{Options, PkgGroup, PkgGroups};
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
pub struct ManifestV1_20_0 {
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
impl_pkg_lookup!(crate::parsed::v1_10_0, ManifestV1_20_0);

// You can't derive `Default` because `schema-version` is a `String`,
// which just defaults to an empty string.
impl Default for ManifestV1_20_0 {
    fn default() -> Self {
        Self {
            schema_version: "1.20.0".into(),
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

impl AsTypedOnlyManifest for ManifestV1_20_0 {
    fn as_typed_only(&self) -> crate::Manifest<TypedOnly> {
        Manifest {
            inner: TypedOnly {
                parsed: Parsed::V1_20_0(self.clone()),
            },
        }
    }
}

impl SchemaVersion for ManifestV1_20_0 {
    fn get_schema_version(&self) -> KnownSchemaVersion {
        KnownSchemaVersion::V1_20_0
    }
}

impl ManifestV1_20_0 {
    /// The catalog stability that the packages in `group` resolve against:
    /// the group's own stability, else the environment's
    /// `options.stability`, or `None` to let the catalog pick its default.
    ///
    /// `group` is the name the packages are locked under, so the default
    /// group is [`DEFAULT_GROUP_NAME`].
    pub fn group_stability(&self, group: &str) -> Option<&str> {
        self.group_stability_override(group)
            .or(self.options.stability.as_deref())
    }

    /// The stability that `group` sets under `[pkg-groups.<group>]`, which
    /// overrides `options.stability` for that group.
    pub fn group_stability_override(&self, group: &str) -> Option<&str> {
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

/// A map of package ids to package build descriptors.
///
/// This is a version-specific copy of the V1_13_0 `Build` because V1_20_0 adds
/// the `auto-patchelf` field to [BuildDescriptor]; the map is otherwise
/// identical.
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

/// The definition of a package built from within the environment.
///
/// V1_20_0 adds `auto-patchelf`: whether the package builder rewrites the
/// `RUNPATH` of ELF files in the build result so that their shared library
/// dependencies resolve from the build's runtime environment. Otherwise
/// identical to the V1_13_0 `BuildDescriptor`.
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
    /// Whether to run `auto-patchelf` over the build result on Linux, after
    /// the build command and before the executables in `$out/bin` are
    /// wrapped, so that ELF files whose `DT_NEEDED` libraries are only found
    /// in the build's runtime environment (e.g. PyPI wheels with native code)
    /// get a `RUNPATH` that resolves them. Ignored on macOS. Defaults to
    /// `false` because `auto-patchelf` replaces the `RUNPATH` of every ELF
    /// file it visits.
    pub auto_patchelf: Option<bool>,
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

// Conversions from the V1_13_0 types, used by the V1_19_0 -> V1_20_0 migration.
// The new `auto_patchelf` field defaults to None, which is what makes the
// migration lossless.
impl From<crate::parsed::v1_13_0::BuildDescriptor> for BuildDescriptor {
    fn from(descriptor: crate::parsed::v1_13_0::BuildDescriptor) -> Self {
        let crate::parsed::v1_13_0::BuildDescriptor {
            command,
            runtime_packages,
            sandbox,
            sandbox_allow,
            version,
            description,
            license,
        } = descriptor;
        BuildDescriptor {
            command,
            runtime_packages,
            sandbox,
            sandbox_allow,
            auto_patchelf: None,
            version,
            description,
            license,
        }
    }
}

impl From<crate::parsed::v1_13_0::Build> for Build {
    fn from(build: crate::parsed::v1_13_0::Build) -> Self {
        Build(
            build
                .into_inner()
                .into_iter()
                .map(|(id, descriptor)| (id, descriptor.into()))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    fn manifest_with_stabilities(
        options_stability: Option<&str>,
        groups: &[(&str, Option<&str>)],
    ) -> ManifestV1_20_0 {
        let pkg_groups = groups
            .iter()
            .map(|(name, stability)| {
                (name.to_string(), PkgGroup {
                    stability: stability.map(str::to_string),
                })
            })
            .collect();
        ManifestV1_20_0 {
            options: Options {
                stability: options_stability.map(str::to_string),
                ..Default::default()
            },
            pkg_groups: PkgGroups(pkg_groups),
            ..Default::default()
        }
    }

    /// A group's own stability wins over `options.stability`, which applies
    /// to every other group.
    #[test]
    fn group_stability_prefers_group_over_options() {
        let manifest =
            manifest_with_stabilities(Some("stable"), &[("legacy", Some("lts")), ("tools", None)]);

        assert_eq!(
            [
                manifest.group_stability("legacy"),
                manifest.group_stability("tools"),
                manifest.group_stability(DEFAULT_GROUP_NAME),
            ],
            [Some("lts"), Some("stable"), Some("stable")]
        );
    }

    #[test]
    fn group_stability_is_unset_without_group_or_options_stability() {
        let manifest = manifest_with_stabilities(None, &[("legacy", Some("lts"))]);

        assert_eq!(manifest.group_stability(DEFAULT_GROUP_NAME), None);
    }

    /// Pkg-group names that aren't bare TOML keys are quoted, so that a
    /// name with a dot doesn't read as a nested table.
    #[test]
    fn pkg_groups_key_path_quotes_names() {
        assert_eq!(PkgGroups::key_path("legacy"), "pkg-groups.legacy");
        assert_eq!(PkgGroups::key_path("v1.2"), r#"pkg-groups."v1.2""#);
        assert_eq!(PkgGroups::key_path("my group"), r#"pkg-groups."my group""#);
    }

    fn build_descriptor(toml: &str) -> BuildDescriptor {
        toml_edit::de::from_str::<BuildDescriptor>(toml).unwrap()
    }

    /// `auto-patchelf` is unset unless the build asks for it, so the package
    /// builder's default is to leave the build result's ELF files alone.
    #[test]
    fn auto_patchelf_is_unset_by_default() {
        assert_eq!(build_descriptor("command = 'hello'"), BuildDescriptor {
            command: "hello".to_string(),
            runtime_packages: None,
            sandbox: None,
            sandbox_allow: None,
            auto_patchelf: None,
            version: None,
            description: None,
            license: None,
        });
    }

    #[test]
    fn auto_patchelf_parses_alongside_other_build_fields() {
        let descriptor = build_descriptor(indoc! {r#"
            command = 'hello'
            auto-patchelf = true
            sandbox = "off"
            runtime-packages = ["python3"]
        "#});

        assert_eq!(descriptor, BuildDescriptor {
            command: "hello".to_string(),
            runtime_packages: Some(vec!["python3".to_string()]),
            sandbox: Some(BuildSandbox::Off),
            sandbox_allow: None,
            auto_patchelf: Some(true),
            version: None,
            description: None,
            license: None,
        });
    }
}
