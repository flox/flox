use std::collections::{BTreeMap, HashMap};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use flox_core::Version;
use indoc::{formatdoc, indoc};
use itertools::Itertools;
#[cfg(test)]
use proptest::prelude::*;
use serde::de::Error;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use toml_edit::{self, Array, DocumentMut, Formatted, InlineTable, Item, Key, Table, Value};
use tracing::{debug, trace};
use url::Url;

use super::environment::path_environment::InitCustomization;
use crate::data::System;
use crate::providers::services::ServiceError;
use crate::utils::gomap::GoMap;
#[cfg(test)]
use crate::utils::proptest_btree_map_alphanum_keys;

pub(super) const DEFAULT_GROUP_NAME: &str = "toplevel";
pub const DEFAULT_PRIORITY: u64 = 5;

/// Represents the `[version]` number key in manifest.toml
pub const MANIFEST_VERSION_KEY: &str = "version";
/// Represents the `[install]` table key in manifest.toml
pub const MANIFEST_INSTALL_KEY: &str = "install";
/// Represents the `[vars]` table key in manifest.toml
pub const MANIFEST_VARS_KEY: &str = "vars";
/// Represents the `[hook]` table key in manifest.toml
pub const MANIFEST_HOOK_KEY: &str = "hook";
/// Represents the `[profile]` table key in manifest.toml
pub const MANIFEST_PROFILE_KEY: &str = "profile";
/// Represents the `[services]` table key in manifest.toml
pub const MANIFEST_SERVICES_KEY: &str = "services";
/// Represents the `[options]` table key in manifest.toml
pub const MANIFEST_OPTIONS_KEY: &str = "options";
/// Represents the `systems = []` array key in manifest.toml
pub const MANIFEST_SYSTEMS_KEY: &str = "systems";

/// A wrapper around a [`toml_edit::DocumentMut`]
/// that allows modifications of the raw manifest document,
/// while preserving comments and user formatting.
#[derive(Clone, Debug)]
pub struct RawManifest(toml_edit::DocumentMut);
impl RawManifest {
    /// Creates a new [RawManifest] instance, populating its configuration from
    /// fields in `customization` [InitCustomization] and systems [System] arguments.
    ///
    /// Additionally, this method prefixes each table with documentation on its usage, and
    /// and inserts commented configuration examples for tables left empty.
    pub fn new_documented(systems: &[&System], customization: &InitCustomization) -> RawManifest {
        let mut manifest = DocumentMut::new();

        manifest.decor_mut().set_prefix(indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##               https://flox.dev/docs/concepts/manifest
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
        "#});

        // `version` number
        manifest.insert(MANIFEST_VERSION_KEY, toml_edit::value(1));

        // `[install]` table
        let packages_vec = vec![];
        let packages = customization.packages.as_ref().unwrap_or(&packages_vec);

        let mut install_table = if packages.is_empty() {
            // Add comment with example packages
            let mut table = Table::new();

            table.decor_mut().set_suffix(indoc! {r#"

                # gum.pkg-path = "gum"
                # gum.version = "^0.14.5""#
            });

            table
        } else {
            Table::from_iter(packages.iter().map(|pkg| (&pkg.id, InlineTable::from(pkg))))
        };

        install_table.decor_mut().set_prefix(indoc! {r#"


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
        "#});

        manifest.insert(MANIFEST_INSTALL_KEY, Item::Table(install_table));

        // `[vars]` table
        let mut vars_table = Table::new();

        vars_table.decor_mut().set_prefix(indoc! {r#"


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
        "#});

        // [sic]: vars not customized using InitCustomization yet
        vars_table.decor_mut().set_suffix(indoc! {r#"

            # INTRO_MESSAGE = "It's gettin' Flox in here""#});

        manifest.insert(MANIFEST_VARS_KEY, Item::Table(vars_table));

        // `[hook]` table
        let mut hook_table = Table::new();

        hook_table.decor_mut().set_prefix(indoc! {r#"


            ## Activation Hook ---------------------------------------------------
            ##  ... run by _bash_ shell when you run 'flox activate'.
            ## -------------------------------------------------------------------
        "#});

        if let Some(ref hook_on_activate_script) = customization.hook_on_activate {
            let on_activate_content = indent::indent_all_by(2, hook_on_activate_script);

            hook_table.insert("on-activate", toml_edit::value(on_activate_content));
        } else {
            hook_table.decor_mut().set_suffix(indoc! {r#"

                # on-activate = '''
                #   # -> Set variables, create files and directories
                #   # -> Perform initialization steps, e.g. create a python venv
                #   # -> Useful environment variables:
                #   #      - FLOX_ENV_PROJECT=/home/user/example
                #   #      - FLOX_ENV=/home/user/example/.flox/run
                #   #      - FLOX_ENV_CACHE=/home/user/example/.flox/cache
                # '''"#
            });
        };

        manifest.insert(MANIFEST_HOOK_KEY, Item::Table(hook_table));

        // `[profile]` table
        let mut profile_table = Table::new();

        profile_table.decor_mut().set_prefix(indoc! {r#"


            ## Profile script ----------------------------------------------------
            ## ... sourced by _your shell_ when you run 'flox activate'.
            ## -------------------------------------------------------------------
        "#});

        match customization {
            InitCustomization {
                profile_common: None,
                profile_bash: None,
                profile_fish: None,
                profile_tcsh: None,
                profile_zsh: None,
                ..
            } => {
                profile_table.decor_mut().set_suffix(indoc! {r#"

                    # common = '''
                    #   gum style \
                    #   --foreground 212 --border-foreground 212 --border double \
                    #   --align center --width 50 --margin "1 2" --padding "2 4" \
                    #     $INTRO_MESSAGE
                    # '''
                    ## Shell specific profiles go here:
                    # bash = ...
                    # zsh  = ...
                    # fish = ..."#
                });
            },
            _ => {
                if let Some(profile_common) = &customization.profile_common {
                    profile_table.insert(
                        "common",
                        toml_edit::value(indent::indent_all_by(2, profile_common)),
                    );
                }
                if let Some(profile_bash) = &customization.profile_bash {
                    profile_table.insert(
                        "bash",
                        toml_edit::value(indent::indent_all_by(2, profile_bash)),
                    );
                }
                if let Some(profile_fish) = &customization.profile_fish {
                    profile_table.insert(
                        "fish",
                        toml_edit::value(indent::indent_all_by(2, profile_fish)),
                    );
                }
                if let Some(profile_tcsh) = &customization.profile_tcsh {
                    profile_table.insert(
                        "tcsh",
                        toml_edit::value(indent::indent_all_by(2, profile_tcsh)),
                    );
                }
                if let Some(profile_zsh) = &customization.profile_zsh {
                    profile_table.insert(
                        "zsh",
                        toml_edit::value(indent::indent_all_by(2, profile_zsh)),
                    );
                }
            },
        };

        manifest.insert(MANIFEST_PROFILE_KEY, Item::Table(profile_table));

        // `[services]` table
        let mut services_table = Table::new();

        services_table.decor_mut().set_prefix(indoc! {r#"


                ## Services ----------------------------------------------------------
                ##  $ flox services start             <- Starts all services
                ##  $ flox services status            <- Status of running services
                ##  $ flox activate --start-services  <- Activates & starts all
                ## -------------------------------------------------------------------
            "#});

        services_table.decor_mut().set_suffix(indoc! {r#"

                # myservice.command = "python3 -m http.server""#});

        manifest.insert(MANIFEST_SERVICES_KEY, Item::Table(services_table));

        // `[options]` table
        let mut options_table = Table::new();

        options_table.decor_mut().set_prefix(indoc! {r#"


            ## Other Environment Options -----------------------------------------
        "#});

        // `systems` array with custom formatting
        let mut systems_array = Array::new();
        for system in systems {
            let mut item = Value::from(system.to_string());
            item.decor_mut().set_prefix("\n  "); // Indent each item with two spaces
            if Some(system) == systems.last() {
                item.decor_mut().set_suffix(",\n"); // Add a newline before the first item
            }
            systems_array.push_formatted(item);
        }

        let systems_key = Key::new(MANIFEST_SYSTEMS_KEY);
        options_table.insert(&systems_key, toml_edit::value(systems_array));
        if let Some((mut key, _)) = options_table.get_key_value_mut(&systems_key) {
            key.leaf_decor_mut().set_prefix(indoc! {r#"
                # Systems that environment is compatible with
                "#});
        }

        let cuda_detection_key = Key::new("cuda-detection");
        options_table.insert(&cuda_detection_key, toml_edit::value(false));
        if let Some((mut key, _)) = options_table.get_key_value_mut(&cuda_detection_key) {
            key.leaf_decor_mut().set_prefix(indoc! {r#"
            # Uncomment to disable CUDA detection.
            # "#});
        }

        manifest.insert(MANIFEST_OPTIONS_KEY, Item::Table(options_table));

        RawManifest(manifest)
    }

    /// Get the version of the manifest.
    fn get_version(&self) -> Option<i64> {
        self.0.get("version").and_then(Item::as_integer)
    }

    /// Serde's error messages for _untagged_ enums are rather bad
    /// and don't appear to become better any time soon:
    /// - <https://github.com/serde-rs/serde/pull/1544>
    /// - <https://github.com/serde-rs/serde/pull/2376>
    ///
    /// This function aims to provide the intermediate version matching on
    /// the `version` field, and then deserializes the correct version
    /// of the Manifest explicitly.
    ///
    /// <https://github.com/serde-rs/serde/pull/2525> will allow the use of integers
    /// (i.e. versions) as enum tags, which will allow us to use `#[serde(tag = "version")]`
    /// and avoid the [Version] field entirely, where the version field is not optional.
    ///
    /// Discussion: using a string field as the version tag `version: "1"` vs `version: 1`
    /// could work today, but is still limited by the lack of an optional tag.
    pub fn to_typed(&self) -> Result<Manifest, toml_edit::de::Error> {
        match self.get_version() {
            Some(1) => Ok(toml_edit::de::from_document(self.0.clone())?),
            Some(v) => {
                let msg = format!("unsupported manifest version: {v}");
                Err(toml_edit::de::Error::custom(msg))
            },
            None => Err(toml_edit::de::Error::custom("unsupported manifest version")),
        }
    }
}

impl FromStr for RawManifest {
    type Err = toml_edit::de::Error;

    /// Parses a string to a `ManifestMut` and validates that it's a valid manifest
    /// Validation is currently only checking the structure of the manifest,
    /// not the precise contents.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let doc = s.parse::<DocumentMut>()?;
        let manifest = RawManifest(doc);
        let _validate = manifest.to_typed()?;
        Ok(manifest)
    }
}

impl Deref for RawManifest {
    type Target = DocumentMut;

    // Allows accessing the [DocumentMut] instance wrapped by [RawManifest].
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for RawManifest {
    // Allows accessing the mutable [DocumentMut] instance wrapped by [RawManifest].
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Not meant for writing manifest files, only for reading them.
/// Modifications should be made using the the raw functions in this module.

// We use skip_serializing_if throughout to reduce the size of the lockfile and
// improve backwards compatibility when we introduce fields.
// We don't use Option and skip_serializing_none because an empty table gets
// treated as Some,
// but we don't care about distinguishing between a table not being present and
// a table being present but empty.
// In both cases, we can just skip serializing.
// It would be better if we could deny_unknown_fields when we're deserializing
// the user provided manifest but allow unknown fields when deserializing the
// lockfile,
// but that doesn't seem worth the effort at the moment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: Version<1>,
    /// The packages to install in the form of a map from install_id
    /// to package descriptor.
    #[serde(default)]
    #[serde(skip_serializing_if = "ManifestInstall::skip_serializing")]
    pub install: ManifestInstall,
    /// Variables that are exported to the shell environment upon activation.
    #[serde(default)]
    #[serde(skip_serializing_if = "ManifestVariables::skip_serializing")]
    pub vars: ManifestVariables,
    /// Hooks that are run at various times during the lifecycle of the manifest
    /// in a known shell environment.
    #[serde(default)]
    pub hook: ManifestHook,
    /// Profile scripts that are run in the user's shell upon activation.
    #[serde(default)]
    pub profile: ManifestProfile,
    /// Options that control the behavior of the manifest.
    #[serde(default)]
    pub options: ManifestOptions,
    /// Service definitions
    #[serde(default)]
    #[serde(skip_serializing_if = "ManifestServices::skip_serializing")]
    pub services: ManifestServices,
    /// Package build definitions
    #[serde(default)]
    #[serde(skip_serializing_if = "ManifestBuild::skip_serializing")]
    pub build: ManifestBuild,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub containerize: Option<ManifestContainerize>,
}

impl Manifest {
    /// Get the package descriptor with the specified install_id.
    pub fn pkg_descriptor_with_id(&self, id: impl AsRef<str>) -> Option<ManifestPackageDescriptor> {
        self.install.0.get(id.as_ref()).cloned()
    }

    /// Get the package descriptor with the specified install_id.
    pub fn catalog_pkg_descriptor_with_id(
        &self,
        id: impl AsRef<str>,
    ) -> Option<ManifestPackageDescriptorCatalog> {
        self.install
            .0
            .get(id.as_ref())
            .and_then(ManifestPackageDescriptor::as_catalog_descriptor_ref)
            .cloned()
    }

    /// Get the package descriptor with the specified install_id.
    pub fn flake_pkg_descriptor_with_id(
        &self,
        id: impl AsRef<str>,
    ) -> Option<ManifestPackageDescriptor> {
        self.install.0.get(id.as_ref()).cloned()
    }

    /// Get the package descriptors in the "toplevel" group.
    pub fn pkg_descriptors_in_toplevel_group(&self) -> Vec<(String, ManifestPackageDescriptor)> {
        pkg_descriptors_in_toplevel_group(&self.install.0)
    }

    /// Get the package descriptors in a named group.
    pub fn pkg_descriptors_in_named_group(
        &self,
        name: impl AsRef<str>,
    ) -> Vec<(String, ManifestPackageDescriptor)> {
        pkg_descriptors_in_named_group(name, &self.install.0)
    }

    /// Check whether the specified name is either an install_id or group name.
    pub fn pkg_or_group_found_in_manifest(&self, name: impl AsRef<str>) -> bool {
        pkg_or_group_found_in_manifest(name.as_ref(), &self.install.0)
    }

    /// Check whether the specified package belongs to a named group
    /// with additional packages.
    pub fn pkg_belongs_to_non_empty_named_group(
        &self,
        pkg: impl AsRef<str>,
    ) -> Result<Option<String>, ManifestError> {
        pkg_belongs_to_non_empty_named_group(pkg.as_ref(), &self.install.0)
    }

    /// Check whether the specified package belongs to the "toplevel" group
    /// with additional packages.
    pub fn pkg_belongs_to_non_empty_toplevel_group(
        &self,
        pkg: impl AsRef<str>,
    ) -> Result<bool, ManifestError> {
        pkg_belongs_to_non_empty_toplevel_group(pkg.as_ref(), &self.install.0)
    }
}

pub(crate) fn pkg_descriptors_in_toplevel_group(
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> Vec<(String, ManifestPackageDescriptor)> {
    descriptors
        .iter()
        .filter(|(_, desc)| {
            let ManifestPackageDescriptor::Catalog(ManifestPackageDescriptorCatalog {
                pkg_group,
                ..
            }) = desc
            else {
                return false;
            };

            pkg_group.is_none()
        })
        .map(|(id, desc)| (id.clone(), desc.clone()))
        .collect::<Vec<_>>()
}

pub(crate) fn pkg_descriptors_in_named_group(
    name: impl AsRef<str>,
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> Vec<(String, ManifestPackageDescriptor)> {
    descriptors
        .iter()
        .filter(|(_, desc)| {
            let ManifestPackageDescriptor::Catalog(ManifestPackageDescriptorCatalog {
                pkg_group,
                ..
            }) = desc
            else {
                return false;
            };

            pkg_group
                .as_ref()
                .is_some_and(|n| n.as_str() == name.as_ref())
        })
        .map(|(id, desc)| (id.clone(), desc.clone()))
        .collect::<Vec<_>>()
}

/// Scans the provided package descriptors to determine if the search term is a package or
/// group in the manifest.

fn pkg_or_group_found_in_manifest(
    search_term: impl AsRef<str>,
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> bool {
    descriptors.iter().any(|(id, desc)| {
        let group = if let ManifestPackageDescriptor::Catalog(catalog) = desc {
            catalog.pkg_group.as_deref()
        } else {
            None
        };

        let search_term = search_term.as_ref();

        (search_term == id.as_str()) || (Some(search_term) == group)
    })
}

/// named group in the manifest with other packages.
fn pkg_belongs_to_non_empty_named_group(
    pkg: &str,
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> Result<Option<String>, ManifestError> {
    let descriptor = descriptors
        .get(pkg)
        .ok_or(ManifestError::PkgOrGroupNotFound(pkg.to_string()))?;

    let ManifestPackageDescriptor::Catalog(ManifestPackageDescriptorCatalog { pkg_group, .. }) =
        descriptor
    else {
        return Ok(None);
    };

    let Some(ref group) = pkg_group else {
        return Ok(None);
    };
    let pkgs = pkg_descriptors_in_named_group(group, descriptors);
    let other_pkgs_in_group = pkgs.iter().any(|(id, _)| id != pkg);
    if other_pkgs_in_group {
        Ok(Some(group.clone()))
    } else {
        Ok(None)
    }
}

/// Scans the provided package descriptors to determine if the specified package belongs to
/// the "toplevel" group with other packages.
fn pkg_belongs_to_non_empty_toplevel_group(
    pkg: &str,
    descriptors: &BTreeMap<String, ManifestPackageDescriptor>,
) -> Result<bool, ManifestError> {
    descriptors
        .get(pkg)
        .ok_or(ManifestError::PkgOrGroupNotFound(pkg.to_string()))?;
    let pkgs = pkg_descriptors_in_toplevel_group(descriptors);
    let other_toplevel_packages_exist = pkgs.iter().any(|(id, _)| id != pkg);
    Ok(other_toplevel_packages_exist)
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    PartialEq,
    derive_more::Deref,
    derive_more::DerefMut,
)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestInstall(
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest_btree_map_alphanum_keys::<ManifestPackageDescriptor>(10, 3)"
        )
    )]
    BTreeMap<String, ManifestPackageDescriptor>,
);

impl ManifestInstall {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
// todo: this can make the error messages less clear and might call for a custom (de)serialize impl
#[serde(
    untagged,
    expecting = "Expected either a catalog package descriptor, a flake installable or a store path.
See https://flox.dev/docs/concepts/manifest/#package-descriptors for more information."
)]
pub enum ManifestPackageDescriptor {
    Catalog(ManifestPackageDescriptorCatalog),
    FlakeRef(ManifestPackageDescriptorFlake),
    StorePath(ManifestPackageDescriptorStorePath),
}

impl ManifestPackageDescriptor {
    /// Check if two package descriptors should have the same resolution.
    /// This is used to determine if a package needs to be re-resolved
    /// in the presence of an existing lock.
    ///
    /// * Descriptors are resolved per system,
    ///   changing the supported systems does not invalidate _existing_ resolutions.
    /// * Priority is not used in resolution, so it is ignored.
    pub(super) fn invalidates_existing_resolution(&self, other: &Self) -> bool {
        use ManifestPackageDescriptor::*;
        match (self, other) {
            (Catalog(this), Catalog(other)) => this.invalidates_existing_resolution(other),
            (FlakeRef(this), FlakeRef(other)) => this != other,
            // different types of descriptors are always different
            _ => true,
        }
    }

    #[must_use]
    pub fn unwrap_catalog_descriptor(self) -> Option<ManifestPackageDescriptorCatalog> {
        match self {
            ManifestPackageDescriptor::Catalog(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_catalog_descriptor_ref(&self) -> Option<&ManifestPackageDescriptorCatalog> {
        match self {
            ManifestPackageDescriptor::Catalog(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn unwrap_flake_descriptor(self) -> Option<ManifestPackageDescriptorFlake> {
        match self {
            ManifestPackageDescriptor::FlakeRef(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_flake_descriptor_ref(&self) -> Option<&ManifestPackageDescriptorFlake> {
        match self {
            ManifestPackageDescriptor::FlakeRef(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn unwrap_store_path_descriptor(self) -> Option<ManifestPackageDescriptorStorePath> {
        match self {
            ManifestPackageDescriptor::StorePath(descriptor) => Some(descriptor),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_store_path_descriptor_ref(&self) -> Option<&ManifestPackageDescriptorStorePath> {
        match self {
            ManifestPackageDescriptor::StorePath(descriptor) => Some(descriptor),
            _ => None,
        }
    }
}

impl From<&ManifestPackageDescriptorCatalog> for ManifestPackageDescriptor {
    fn from(val: &ManifestPackageDescriptorCatalog) -> Self {
        ManifestPackageDescriptor::Catalog(val.clone())
    }
}

impl From<ManifestPackageDescriptorCatalog> for ManifestPackageDescriptor {
    fn from(val: ManifestPackageDescriptorCatalog) -> Self {
        ManifestPackageDescriptor::Catalog(val)
    }
}

impl From<&ManifestPackageDescriptorFlake> for ManifestPackageDescriptor {
    fn from(val: &ManifestPackageDescriptorFlake) -> Self {
        ManifestPackageDescriptor::FlakeRef(val.clone())
    }
}

impl From<ManifestPackageDescriptorFlake> for ManifestPackageDescriptor {
    fn from(val: ManifestPackageDescriptorFlake) -> Self {
        ManifestPackageDescriptor::FlakeRef(val)
    }
}

impl From<&ManifestPackageDescriptorStorePath> for ManifestPackageDescriptor {
    fn from(val: &ManifestPackageDescriptorStorePath) -> Self {
        ManifestPackageDescriptor::StorePath(val.clone())
    }
}

impl From<ManifestPackageDescriptorStorePath> for ManifestPackageDescriptor {
    fn from(val: ManifestPackageDescriptorStorePath) -> Self {
        ManifestPackageDescriptor::StorePath(val)
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestPackageDescriptorCatalog {
    pub(crate) pkg_path: String,
    pub(crate) pkg_group: Option<String>,
    #[cfg_attr(test, proptest(strategy = "proptest::option::of(0..10u64)"))]
    pub(crate) priority: Option<u64>,
    pub(crate) version: Option<String>,
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::option::of(proptest::collection::vec(any::<System>(), 1..3))"
        )
    )]
    pub(crate) systems: Option<Vec<System>>,
}

impl ManifestPackageDescriptorCatalog {
    /// Check if two package descriptors should have the same resolution.
    /// This is used to determine if a package needs to be re-resolved
    /// in the presence of an existing lock.
    ///
    /// * Descriptors are resolved per system,
    ///   changing the supported systems does not invalidate _existing_ resolutions.
    /// * Priority is not used in resolution, so it is ignored.
    pub(super) fn invalidates_existing_resolution(&self, other: &Self) -> bool {
        // unpack to avoid forgetting to update this method when new fields are added
        let ManifestPackageDescriptorCatalog {
            pkg_path,
            pkg_group,
            version,
            systems: _,
            priority: _,
        } = self;

        pkg_path != &other.pkg_path || pkg_group != &other.pkg_group || version != &other.version
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestPackageDescriptorFlake {
    pub flake: String,
    #[cfg_attr(test, proptest(strategy = "proptest::option::of(0..10u64)"))]
    pub(crate) priority: Option<u64>,
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::option::of(proptest::collection::vec(any::<System>(), 1..3))"
        )
    )]
    pub(crate) systems: Option<Vec<System>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
#[skip_serializing_none]
pub struct ManifestPackageDescriptorStorePath {
    pub(crate) store_path: String,
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::option::of(proptest::collection::vec(any::<System>(), 1..3))"
        )
    )]
    pub(crate) systems: Option<Vec<System>>,
    #[cfg_attr(test, proptest(strategy = "proptest::option::of(0..10u64)"))]
    pub(crate) priority: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestVariables(
    #[cfg_attr(
        test,
        proptest(strategy = "proptest_btree_map_alphanum_keys::<String>(10, 3)")
    )]
    pub(crate) BTreeMap<String, String>,
);

impl ManifestVariables {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestHook {
    /// A script that is run at activation time,
    /// in a flox provided bash shell
    on_activate: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct ManifestProfile {
    /// When defined, this hook is run by _all_ shells upon activation
    common: Option<String>,
    /// When defined, this hook is run upon activation in a bash shell
    bash: Option<String>,
    /// When defined, this hook is run upon activation in a zsh shell
    zsh: Option<String>,
    /// When defined, this hook is run upon activation in a fish shell
    fish: Option<String>,
    /// When defined, this hook is run upon activation in a tcsh shell
    tcsh: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestOptions {
    /// A list of systems that each package is resolved for.
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest::option::of(proptest::collection::vec(any::<System>(), 1..4))"
        )
    )]
    pub systems: Option<Vec<System>>,
    /// Options that control what types of packages are allowed.
    #[serde(default)]
    pub allow: Allows,
    /// Options that control how semver versions are resolved.
    #[serde(default)]
    pub semver: SemverOptions,
    pub cuda_detection: Option<bool>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(deny_unknown_fields)]
pub struct Allows {
    /// Whether to allow packages that are marked as `unfree`
    pub unfree: Option<bool>,
    /// Whether to allow packages that are marked as `broken`
    pub broken: Option<bool>,
    /// A list of license descriptors that are allowed
    #[serde(default)]
    #[cfg_attr(
        test,
        proptest(strategy = "proptest::collection::vec(any::<String>(), 0..3)")
    )]
    pub licenses: Vec<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct SemverOptions {
    /// Whether to allow pre-release versions when resolving
    #[serde(default)]
    pub allow_pre_releases: Option<bool>,
}

/// A map of service names to service definitions
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    PartialEq,
    derive_more::Deref,
    derive_more::DerefMut,
)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestServices(
    #[cfg_attr(
        test,
        proptest(
            strategy = "proptest_btree_map_alphanum_keys::<ManifestServiceDescriptor>(10, 3)"
        )
    )]
    pub(crate) BTreeMap<String, ManifestServiceDescriptor>,
);

impl ManifestServices {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

/// The definition of a service in a manifest
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestServiceDescriptor {
    /// The command to run to start the service
    pub command: String,
    /// Service-specific environment variables
    pub vars: Option<ManifestVariables>,
    /// Whether the service spawns a background process (daemon)
    // TODO: This option _requires_ the shutdown command, so we'll need to add
    //       that explanation to the manifest.toml docs and service mgmt guide
    pub is_daemon: Option<bool>,
    /// How to shut down the service
    pub shutdown: Option<ManifestServiceShutdown>,
    /// Systems to allow running the service on
    pub systems: Option<Vec<System>>,
}

impl ManifestServices {
    pub fn validate(&self) -> Result<(), ServiceError> {
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
            Err(ServiceError::InvalidConfig(msg))
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
                .map_or(true, |systems| systems.contains(system))
            {
                services.insert(name.clone(), desc.clone());
            }
        }
        ManifestServices(services)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestServiceShutdown {
    /// What command to run to shut down the service
    pub command: String,
}

/// A map of package ids to package build descriptors
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    Default,
    PartialEq,
    derive_more::Deref,
    derive_more::DerefMut,
)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestBuild(
    #[cfg_attr(
        test,
        proptest(strategy = "proptest_btree_map_alphanum_keys::<ManifestBuildDescriptor>(10, 3)")
    )]
    pub(crate) BTreeMap<String, ManifestBuildDescriptor>,
);

impl ManifestBuild {
    fn skip_serializing(&self) -> bool {
        self.0.is_empty()
    }
}

/// The definition of a package built from within the environment
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
#[serde(deny_unknown_fields)]
pub struct ManifestBuildDescriptor {
    /// The command to run to build a package
    pub command: String,
    /// Files to explicitly include in the build result
    pub files: Option<Vec<String>>,
    /// Packages from the 'toplevel' group to include in the closure of the build result
    pub runtime_packages: Option<Vec<String>>,
    /// Systems to allow running the build
    pub systems: Option<Vec<System>>,
    /// Sandbox mode for the build
    pub sandbox: Option<ManifestBuildSandbox>,
}

/// The definition of a package built from within the environment
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, derive_more::Display)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub enum ManifestBuildSandbox {
    Off,
    Pure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestContainerize {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<ManifestContainerizeConfig<Vec<String>>>,
}

/// Container config derived from
/// https://github.com/opencontainers/image-spec/blob/main/config.md
///
/// Env and Entrypoint are left out since they interfere with our activation implementation
/// Deprecated and reserved keys are also left out
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "PascalCase")]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManifestContainerizeConfig<T> {
    /// The username or UID which is a platform-specific structure that allows specific control over which user the process run as.
    /// This acts as a default value to use when the value is not specified when creating a container.
    /// For Linux based systems, all of the following are valid: `user`, `uid`, `user:group`, `uid:gid`, `uid:group`, `user:gid`.
    /// If `group`/`gid` is not specified, the default group and supplementary groups of the given `user`/`uid` in `/etc/passwd` and `/etc/group` from the container are applied.
    /// If `group`/`gid` is specified, supplementary groups from the container are ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    /// A set of ports to expose from a container running this image.
    /// Its keys can be in the format of:
    /// `port/tcp`, `port/udp`, `port` with the default protocol being `tcp` if not specified.
    /// These values act as defaults and are merged with any specified when creating a container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exposed_ports: Option<T>,
    /// Default arguments to the entrypoint of the container.
    /// These values act as defaults and may be replaced by any specified when creating a container.
    /// If an `Entrypoint` value is not specified, then the first entry of the `Cmd` array SHOULD be interpreted as the executable to run.
    #[serde(skip_serializing_if = "Option::is_none")]
    cmd: Option<Vec<String>>,
    /// A set of directories describing where the process is
    /// likely to write data specific to a container instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    volumes: Option<T>,
    /// Sets the current working directory of the entrypoint process in the container.
    /// This value acts as a default and may be replaced by a working directory specified when creating a container.
    #[serde(skip_serializing_if = "Option::is_none")]
    working_dir: Option<String>,
    /// The field contains arbitrary metadata for the container.
    /// This property MUST use the [annotation rules](https://github.com/opencontainers/image-spec/blob/main/annotations.md#rules).
    #[serde(skip_serializing_if = "Option::is_none")]
    labels: Option<BTreeMap<String, String>>,
    /// The field contains the system call signal that will be sent to the container to exit. The signal can be a signal name in the format `SIGNAME`, for instance `SIGKILL` or `SIGRTMIN+3`.
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_signal: Option<String>,
}

impl From<ManifestContainerizeConfig<Vec<String>>> for ManifestContainerizeConfig<GoMap> {
    fn from(config: ManifestContainerizeConfig<Vec<String>>) -> Self {
        ManifestContainerizeConfig {
            exposed_ports: config.exposed_ports.clone().map(GoMap::new),
            volumes: config.volumes.clone().map(GoMap::new),
            ..config.into()
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("couldn't parse descriptor '{}': {}", desc, msg)]
    MalformedStringDescriptor { msg: String, desc: String },
    #[error("no package or group named '{0}' in the manifest")]
    PkgOrGroupNotFound(String),
    #[error("invalid flake ref: {0}")]
    InvalidFlakeRef(String),
    #[error("only remote flake refs are supported: {0}")]
    LocalFlakeRef(String),
}

/// An error encountered while manipulating a manifest using toml_edit.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TomlEditError {
    /// The provided string couldn't be parsed into a valid TOML document
    #[error("couldn't parse manifest contents: {0}")]
    ParseManifest(toml_edit::de::Error),
    /// The provided string was a valid TOML file, but it didn't have
    /// the format that we anticipated.
    #[error("'install' must be a table, but found {0} instead")]
    MalformedInstallTable(String),
    /// The `[install]` table was missing entirely
    #[error("'install' table not found")]
    MissingInstallTable,
    #[error("couldn't find package with install id '{0}' in the manifest")]
    PackageNotFound(String),
    #[error("'options' must be a table, but found {0} instead")]
    MalformedOptionsTable(String),
    #[error("'options' must be an array, but found {0} instead")]
    MalformedOptionsSystemsArray(String),

    #[error("'{0}' is not a supported attribute in manifest version 1")]
    UnsupportedAttributeV1(String),
}

/// Records the result of trying to install a collection of packages to the
#[derive(Debug)]
pub struct PackageInsertion {
    pub new_toml: Option<DocumentMut>,
    pub already_installed: HashMap<String, bool>,
}

/// Any kind of package that can be installed via `flox install`.
#[derive(Debug, Clone, PartialEq)]
pub enum PackageToInstall {
    Catalog(CatalogPackage),
    Flake(FlakePackage),
    StorePath(StorePath),
}

impl PackageToInstall {
    pub fn id(&self) -> &str {
        match self {
            PackageToInstall::Catalog(pkg) => &pkg.id,
            PackageToInstall::Flake(pkg) => &pkg.id,
            PackageToInstall::StorePath(pkg) => &pkg.id,
        }
    }

    pub fn set_id(&mut self, id: impl AsRef<str>) {
        let id = String::from(id.as_ref());
        match self {
            PackageToInstall::Catalog(pkg) => pkg.id = id,
            PackageToInstall::Flake(pkg) => pkg.id = id,
            PackageToInstall::StorePath(pkg) => pkg.id = id,
        }
    }

    /// Parse a package descriptor from a string, inferring the type of package to install.
    /// If the string starts with a path like prefix, it's parsed as a store path,
    /// if it parses as a url, it's assumed to be a flake ref,
    /// otherwise it's parsed as a catalog package.
    ///
    /// The method takes a `system` argument, for which to expect store paths to be valid.
    /// Unlike flake refs, and catalog packages,
    /// store paths are typically only valid on the system they were built for.
    pub fn parse(system: &System, s: &str) -> Result<Self, ManifestError> {
        // if the string starts with a path like prefix, parse it as a store path
        if ["../", "./", "/"]
            .iter()
            .any(|prefix| s.starts_with(prefix))
        {
            return Ok(PackageToInstall::StorePath(StorePath::parse(system, s)?));
        }

        // if the string parses as a url, assume it's a flake ref
        match Url::parse(s) {
            Ok(url) => {
                let id = infer_flake_install_id(&url)?;
                Ok(PackageToInstall::Flake(FlakePackage { id, url }))
            },
            // if it's not a url, parse it as a catalog package
            _ => Ok(PackageToInstall::Catalog(s.parse()?)),
        }
    }
}

/// Tries to infer an install id from the flake ref URL, or falls back to "flake".
fn infer_flake_install_id(url: &Url) -> Result<String, ManifestError> {
    if let Some(fragment) = url.fragment() {
        let fragment = url_escape::decode(fragment).to_string();
        let attr_path = fragment
            // split off extended output spec
            .rsplit_once('^')
            .map(|(attr_path, _)| attr_path.to_string())
            .unwrap_or(fragment);
        if !attr_path.is_empty() {
            let install_id = install_id_from_attr_path(&attr_path, url.as_ref())?;
            return Ok(install_id);
        }
    }

    // Use `.path()`` because `github:` and co. are `cannot-be-a-base` urls
    // for which "path-segments" are undefined.
    // `Url::path_segments` will return `None` for such urls.
    if url.scheme() == "github" {
        // Using `.last()` isn't reliable for `github:` refs because you can have a `/<rev>`
        // after the repository name.
        url.path()
            .split('/')
            .nth(1)
            .map(|s| url_escape::decode(s).to_string())
            .ok_or(ManifestError::InvalidFlakeRef(url.to_string()))
    } else {
        url.path()
            .split('/')
            .last()
            .map(|s| url_escape::decode(s).to_string())
            .ok_or(ManifestError::InvalidFlakeRef(url.to_string()))
    }
}

/// Extracts only the catalog packages from a list of packages to install.
pub fn catalog_packages_to_install(packages: &[PackageToInstall]) -> Vec<CatalogPackage> {
    packages
        .iter()
        .filter_map(|pkg| match pkg {
            PackageToInstall::Catalog(pkg) => Some((*pkg).clone()),
            _ => None,
        })
        .collect()
}

/// A package to install from the catalog.
///
/// Users may specify a different install ID than the package name,
/// especially when the package is nested. This struct is the common
/// denominator for packages with specified IDs and packages with
/// default IDs.
#[derive(Debug, Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CatalogPackage {
    pub id: String,
    pub pkg_path: String,
    pub version: Option<String>,
    /// Systems to resolve the package for.
    /// If `None`, the package is resolved for all systems.
    /// Currently this is not parsed from a shorthand descriptor,
    /// but callers of [Environment::install] can set it
    /// to avoid resolution errors.
    ///
    /// [Environment::install]: crate::models::environment::Environment::install
    pub systems: Option<Vec<System>>,
}

impl FromStr for CatalogPackage {
    type Err = ManifestError;

    /// Parse a shorthand descriptor into `install_id`, `attribute_path` and `version`.
    ///
    /// A shorthand descriptor consists of a package name and an optional version.
    /// The attribute path is a dot-separated path to a package in the catalog.
    /// The last component of the attribute path is the `install_id`.
    ///
    /// The descriptor is parsed as follows:
    /// ```text
    ///     descriptor ::= <attribute_path>[@<version>]
    ///
    ///     attribute_path ::= <install_id> | <attribute_path_rest>.<install_id>
    ///     attribute_path_rest ::= <identifier> | <attribute_path_rest>.<identifier>
    ///     install_id ::= <identifier> | @<identifier>
    ///
    ///     version ::= <string> # interpreted as semver or plain version by the resolver
    /// ```
    fn from_str(descriptor: &str) -> Result<Self, ManifestError> {
        fn split_version(haystack: &str) -> (usize, Option<&str>) {
            let mut version_at = None;
            let mut start = 0;

            loop {
                trace!(descriptor = haystack, start, substring = &haystack[start..]);
                match haystack[start..].find('@') {
                    // Found "@" at the beginning of the descriptor,
                    // interpreted the "@" as part of the first attribute.
                    Some(next_version_at) if start + next_version_at == 0 => {
                        start += 1;
                        continue;
                    },
                    // Found ".@", interpreted the "@" as part of the attribute,
                    // as it would otherwise be unclear what is being versioned.
                    // An example of this is `nodePackages.@angular/cli`
                    Some(next_version_at)
                        if &haystack[start + next_version_at - 1..start + next_version_at]
                            == "." =>
                    {
                        start = start + next_version_at + 1;
                        continue;
                    },
                    // Found a version delimiting "@"
                    Some(next_version_at) => {
                        version_at = Some(start + next_version_at);
                        break;
                    },
                    // No version delimiting "@" found
                    None => break,
                }
            }

            let version = version_at.map(|at| &haystack[at + 1..]);
            (version_at.unwrap_or(haystack.len()), version)
        }

        let (attr_path_len, version) = split_version(descriptor);
        let attr_path = descriptor[..attr_path_len].to_string();
        let version = if let Some(version) = version {
            if version.is_empty() {
                return Err(ManifestError::MalformedStringDescriptor {
                    msg: indoc! {"
                        Expected version requirement after '@'.
                        Try adding quotes around the argument."}
                    .to_string(),
                    desc: descriptor.to_string(),
                });
            }
            Some(version.to_string())
        } else {
            None
        };

        let install_id = install_id_from_attr_path(&attr_path, descriptor)?;

        Ok(Self {
            id: install_id,
            pkg_path: attr_path,
            version,
            systems: None,
        })
    }
}

#[derive(Debug, Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FlakePackage {
    pub id: String,
    pub url: Url,
}

#[derive(Debug, Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StorePath {
    pub id: String,
    pub store_path: PathBuf,
    pub system: System,
}

impl StorePath {
    fn parse(system: &System, descriptor: &str) -> Result<Self, ManifestError> {
        // Don't canonicalize the path if it's already a store path.
        // Canonicalizing a store path can potentially resolve it to a different path,
        // if the original path is a symlink to another store path.
        let path = if Path::new(descriptor).starts_with("/nix/store") {
            PathBuf::from(descriptor)
        } else {
            Path::new(descriptor).canonicalize().map_err(|e| {
                ManifestError::MalformedStringDescriptor {
                    msg: format!("cannot resolve path: {}", e),
                    desc: descriptor.to_string(),
                }
            })?
        };

        // [sic] 4 components, because the root dir is counted as a component on its own
        let store_path: PathBuf = path.components().take(4).collect();
        let Ok(hash_and_name) = store_path.strip_prefix("/nix/store") else {
            return Err(ManifestError::MalformedStringDescriptor {
                msg: "store path must be in the '/nix/store' directory".to_string(),
                desc: descriptor.to_string(),
            });
        };

        // The store path is expected to have the format `<hash>-<name>[-<version>]`
        // the version is not required, but canonically present in store paths derived from nixpkgs.
        //
        // The name is parsed according to the reference implementation in nix
        //
        // > The `name' part of a derivation name is everything up to
        // > but not including the first dash *not* followed by a letter.
        // > The `version' part is the rest (excluding the separating dash).
        // > E.g., `apache-httpd-2.0.48' is parsed to (`apache-httpd', '2.0.48').
        // >
        // > <https://github.com/NixOS/nix/blob/fa17927d9d75b6feec38a3fbc8b6e34e17c71b52/src/libstore/names.cc#L22-L38>
        let id = hash_and_name
            .to_string_lossy()
            .split('-')
            .skip(1)
            .take_while(|component| {
                component
                    .chars()
                    .next()
                    .map(|c| !c.is_ascii_digit())
                    .unwrap_or(true)
            })
            .join("-");

        if id.is_empty() {
            return Err(ManifestError::MalformedStringDescriptor {
                msg: "store path must contain a package name".to_string(),
                desc: store_path.display().to_string(),
            });
        }

        Ok(Self {
            id,
            store_path,
            system: system.clone(),
        })
    }
}

/// Extracts an install ID from a dot-separated attribute path that potentially contains quotes.
fn install_id_from_attr_path(attr_path: &str, descriptor: &str) -> Result<String, ManifestError> {
    let mut install_id = None;
    let mut cur = String::new();

    let mut start_quote = None;

    for (n, c) in attr_path.chars().enumerate() {
        match c {
            '.' if start_quote.is_none() => {
                let _ = install_id.insert(std::mem::take(&mut cur));
            },
            // '"' if start_quote.is_some() => start_quote = None,
            '"' if start_quote.is_some() => {
                start_quote = None;
                cur.push('"');
            },
            // '"' if start_quote.is_none() => start_quote = Some(n),
            '"' if start_quote.is_none() => {
                start_quote = Some(n);
                cur.push('"');
            },
            other => cur.push(other),
        }
    }

    if start_quote.is_some() {
        return Err(ManifestError::MalformedStringDescriptor {
            msg: "unclosed quote".to_string(),
            desc: descriptor.to_string(),
        });
    }

    if !cur.is_empty() {
        let _ = install_id.insert(cur);
    }

    install_id.ok_or(ManifestError::MalformedStringDescriptor {
        msg: "attribute path is empty".to_string(),
        desc: descriptor.to_string(),
    })
}

impl From<&CatalogPackage> for InlineTable {
    fn from(val: &CatalogPackage) -> Self {
        let mut table = InlineTable::new();
        table.insert(
            "pkg-path",
            Value::String(Formatted::new(val.pkg_path.clone())),
        );
        if let Some(ref version) = val.version {
            table.insert("version", Value::String(Formatted::new(version.clone())));
        }
        if let Some(ref systems) = val.systems {
            table.insert(
                "systems",
                Value::Array(
                    systems
                        .iter()
                        .map(|s| Value::String(Formatted::new(s.to_string())))
                        .collect(),
                ),
            );
        }
        table
    }
}

/// Insert package names into the `[install]` table of a manifest.
///
/// Note that the packages may be provided as dot-separated attribute paths
/// that should be interpreted as relative paths under whatever input they're
/// coming from. For this reason we put them under the `<descriptor>.path` key
/// rather than `<descriptor>.name`.
pub fn insert_packages(
    manifest_contents: &str,
    pkgs: &[PackageToInstall],
) -> Result<PackageInsertion, TomlEditError> {
    debug!("attempting to insert packages into manifest");
    let mut already_installed: HashMap<String, bool> = HashMap::new();
    let manifest = manifest_contents
        .parse::<RawManifest>()
        .map_err(TomlEditError::ParseManifest)?;

    let mut toml = manifest.0;

    let install_table = {
        let install_field = toml
            .entry("install")
            .or_insert_with(|| Item::Table(Table::new()));
        let install_field_type = install_field.type_name().into();
        install_field.as_table_mut().ok_or_else(|| {
            debug!("creating new [install] table");
            TomlEditError::MalformedInstallTable(install_field_type)
        })?
    };

    for pkg in pkgs {
        if !install_table.contains_key(pkg.id()) {
            let mut descriptor_table = InlineTable::new();
            match pkg {
                PackageToInstall::Catalog(pkg) => {
                    descriptor_table = InlineTable::from(pkg);
                    debug!(
                        "package newly installed: id={}, pkg-path={}",
                        pkg.id, pkg.pkg_path
                    );
                },
                PackageToInstall::Flake(pkg) => {
                    descriptor_table
                        .insert("flake", Value::String(Formatted::new(pkg.url.to_string())));
                    debug!(
                        "package newly installed: id={}, flakeref={}",
                        pkg.id,
                        pkg.url.to_string()
                    );
                },
                PackageToInstall::StorePath(pkg) => {
                    descriptor_table.insert(
                        "store-path",
                        Value::String(Formatted::new(pkg.store_path.to_string_lossy().to_string())),
                    );
                    descriptor_table.insert(
                        "systems",
                        Value::Array(Array::from_iter([Value::String(Formatted::new(
                            pkg.system.to_string(),
                        ))])),
                    );
                    debug!(
                        id=pkg.id, store_path=%pkg.store_path.display(),
                        "store path newly installed",
                    );
                },
            }

            descriptor_table.set_dotted(true);
            install_table.insert(pkg.id(), Item::Value(Value::InlineTable(descriptor_table)));
            already_installed.insert(pkg.id().to_string(), false);
        } else {
            already_installed.insert(pkg.id().to_string(), true);
            debug!("package already installed: id={}", pkg.id());
        }
    }

    Ok(PackageInsertion {
        new_toml: if !already_installed.values().all(|p| *p) {
            Some(toml)
        } else {
            None
        },
        already_installed,
    })
}

/// Remove package names from the `[install]` table of a manifest based on their install IDs.
pub fn remove_packages(
    manifest_contents: &str,
    install_ids: &[String],
) -> Result<DocumentMut, TomlEditError> {
    debug!("attempting to remove packages from the manifest");
    let mut toml = manifest_contents
        .parse::<RawManifest>()
        .map_err(TomlEditError::ParseManifest)?
        .0;

    let installs_table = {
        let installs_field = toml
            .get_mut("install")
            .ok_or(TomlEditError::PackageNotFound(install_ids[0].clone()))?;

        let type_name = installs_field.type_name().into();

        installs_field
            .as_table_mut()
            .ok_or(TomlEditError::MalformedInstallTable(type_name))?
    };

    for id in install_ids {
        debug!("checking for presence of package '{id}'");
        if !installs_table.contains_key(id) {
            debug!("package with install id '{id}' wasn't found");
            return Err(TomlEditError::PackageNotFound(id.clone()));
        } else {
            installs_table.remove(id);
            debug!("package with install id '{id}' was removed");
        }
    }

    Ok(toml)
}

/// Check whether a TOML document contains a line declaring that the provided package
/// should be installed.
pub fn contains_package(toml: &DocumentMut, pkg_name: &str) -> Result<bool, TomlEditError> {
    if let Some(installs) = toml.get("install") {
        if let Item::Table(installs_table) = installs {
            Ok(installs_table.contains_key(pkg_name))
        } else {
            Err(TomlEditError::MalformedInstallTable(
                installs.type_name().into(),
            ))
        }
    } else {
        Ok(false)
    }
}

/// Add a `system` to the `[options.systems]` array of a manifest
pub fn add_system(toml: &str, system: &str) -> Result<DocumentMut, TomlEditError> {
    let mut doc = toml
        .parse::<RawManifest>()
        .map_err(TomlEditError::ParseManifest)?
        .0;

    // extract the `[options]` table
    let options_table = doc
        .entry("options")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::default()));
    let options_table_type = options_table.type_name().into();
    let options_table = options_table
        .as_table_like_mut()
        .ok_or(TomlEditError::MalformedOptionsTable(options_table_type))?;

    // extract the `options.systems` array
    let systems_list = options_table
        .entry("systems")
        .or_insert(toml_edit::Item::Value(toml_edit::Value::Array(
            toml_edit::Array::default(),
        )));
    let systems_list_type = systems_list.type_name().into();
    let systems_list =
        systems_list
            .as_array_mut()
            .ok_or(TomlEditError::MalformedOptionsSystemsArray(
                systems_list_type,
            ))?;

    // sanity check that the current system is not already in the list
    if systems_list
        .iter()
        .any(|s| s.as_str().map(|s| s == system).unwrap_or_default())
    {
        debug!("system '{system}' already in 'options.systems'");
        return Ok(doc);
    }

    systems_list.push(system.to_string());

    Ok(doc)
}

#[cfg(test)]
pub(super) mod test {
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use proptest_derive::Arbitrary;

    use super::*;
    use crate::models::lockfile::DEFAULT_SYSTEMS_STR;

    const DUMMY_MANIFEST: &str = indoc! {r#"
        version = 1

        [install]
        hello.pkg-path = "hello"

        [install.ripgrep]
        pkg-path = "ripgrep"
        [install.bat]
        pkg-path = "bat"
    "#};

    // This is an array of tables called `install` rather than a table called `install`.
    const BAD_MANIFEST: &str = indoc! {r#"
        version = 1

        [[install]]
        python = {}

        [[install]]
        ripgrep = {}
    "#};

    const CATALOG_MANIFEST: &str = indoc! {r#"
        version = 1
    "#};

    #[test]
    fn catalog_manifest_rejects_unknown_fields() {
        let manifest = formatdoc! {"
            {CATALOG_MANIFEST}

            unknown = 'field'
        "};

        let err = toml_edit::de::from_str::<Manifest>(&manifest)
            .expect_err("manifest.toml should be invalid");

        assert!(
            err.message()
                .starts_with("unknown field `unknown`, expected one of"),
            "unexpected error message: {err}",
        );
    }

    #[test]
    fn catalog_manifest_rejects_unknown_nested_fields() {
        let manifest = formatdoc! {"
            {CATALOG_MANIFEST}

            [options]
            allow.unknown = true
        "};

        let err = toml_edit::de::from_str::<Manifest>(&manifest)
            .expect_err("manifest.toml should be invalid");

        assert!(
            err.message()
                .starts_with("unknown field `unknown`, expected one of"),
            "unexpected error message: {err}",
        );
    }

    #[test]
    fn create_documented_manifest_not_customized() {
        let systems = &*DEFAULT_SYSTEMS_STR.iter().collect::<Vec<_>>();
        let customization = InitCustomization {
            hook_on_activate: None,
            profile_common: None,
            profile_bash: None,
            profile_fish: None,
            profile_tcsh: None,
            profile_zsh: None,
            packages: None,
        };

        let expected_string = indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##               https://flox.dev/docs/concepts/manifest
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
            version = 1


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
            [install]
            # gum.pkg-path = "gum"
            # gum.version = "^0.14.5"


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
            [vars]
            # INTRO_MESSAGE = "It's gettin' Flox in here"


            ## Activation Hook ---------------------------------------------------
            ##  ... run by _bash_ shell when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [hook]
            # on-activate = '''
            #   # -> Set variables, create files and directories
            #   # -> Perform initialization steps, e.g. create a python venv
            #   # -> Useful environment variables:
            #   #      - FLOX_ENV_PROJECT=/home/user/example
            #   #      - FLOX_ENV=/home/user/example/.flox/run
            #   #      - FLOX_ENV_CACHE=/home/user/example/.flox/cache
            # '''


            ## Profile script ----------------------------------------------------
            ## ... sourced by _your shell_ when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [profile]
            # common = '''
            #   gum style \
            #   --foreground 212 --border-foreground 212 --border double \
            #   --align center --width 50 --margin "1 2" --padding "2 4" \
            #     $INTRO_MESSAGE
            # '''
            ## Shell specific profiles go here:
            # bash = ...
            # zsh  = ...
            # fish = ...


            ## Services ----------------------------------------------------------
            ##  $ flox services start             <- Starts all services
            ##  $ flox services status            <- Status of running services
            ##  $ flox activate --start-services  <- Activates & starts all
            ## -------------------------------------------------------------------
            [services]
            # myservice.command = "python3 -m http.server"


            ## Other Environment Options -----------------------------------------
            [options]
            # Systems that environment is compatible with
            systems = [
              "aarch64-darwin",
              "aarch64-linux",
              "x86_64-darwin",
              "x86_64-linux",
            ]
            # Uncomment to disable CUDA detection.
            # cuda-detection = false
        "#};

        let manifest = RawManifest::new_documented(systems, &customization);
        assert_eq!(manifest.to_string(), expected_string.to_string());
    }

    #[test]
    fn create_documented_manifest_with_packages() {
        let systems = &*DEFAULT_SYSTEMS_STR.iter().collect::<Vec<_>>();
        let customization = InitCustomization {
            hook_on_activate: None,
            profile_common: None,
            profile_bash: None,
            profile_fish: None,
            profile_tcsh: None,
            profile_zsh: None,
            packages: Some(vec![CatalogPackage {
                id: "python3".to_string(),
                pkg_path: "python3".to_string(),
                version: Some("3.11.6".to_string()),
                systems: None,
            }]),
        };

        let expected_string = indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##               https://flox.dev/docs/concepts/manifest
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
            version = 1


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
            [install]
            python3 = { pkg-path = "python3", version = "3.11.6" }


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
            [vars]
            # INTRO_MESSAGE = "It's gettin' Flox in here"


            ## Activation Hook ---------------------------------------------------
            ##  ... run by _bash_ shell when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [hook]
            # on-activate = '''
            #   # -> Set variables, create files and directories
            #   # -> Perform initialization steps, e.g. create a python venv
            #   # -> Useful environment variables:
            #   #      - FLOX_ENV_PROJECT=/home/user/example
            #   #      - FLOX_ENV=/home/user/example/.flox/run
            #   #      - FLOX_ENV_CACHE=/home/user/example/.flox/cache
            # '''


            ## Profile script ----------------------------------------------------
            ## ... sourced by _your shell_ when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [profile]
            # common = '''
            #   gum style \
            #   --foreground 212 --border-foreground 212 --border double \
            #   --align center --width 50 --margin "1 2" --padding "2 4" \
            #     $INTRO_MESSAGE
            # '''
            ## Shell specific profiles go here:
            # bash = ...
            # zsh  = ...
            # fish = ...


            ## Services ----------------------------------------------------------
            ##  $ flox services start             <- Starts all services
            ##  $ flox services status            <- Status of running services
            ##  $ flox activate --start-services  <- Activates & starts all
            ## -------------------------------------------------------------------
            [services]
            # myservice.command = "python3 -m http.server"


            ## Other Environment Options -----------------------------------------
            [options]
            # Systems that environment is compatible with
            systems = [
              "aarch64-darwin",
              "aarch64-linux",
              "x86_64-darwin",
              "x86_64-linux",
            ]
            # Uncomment to disable CUDA detection.
            # cuda-detection = false
        "#};

        let manifest = RawManifest::new_documented(systems, &customization);
        assert_eq!(manifest.to_string(), expected_string.to_string());
    }

    #[test]
    fn create_documented_manifest_hook() {
        let systems = [&"x86_64-linux".to_string()];
        let customization = InitCustomization {
            hook_on_activate: Some(
                indoc! {r#"
                    # Print something
                    echo "hello world"

                    # Set a environment variable
                    $FOO="bar"
                "#}
                .to_string(),
            ),
            profile_common: None,
            profile_bash: None,
            profile_fish: None,
            profile_tcsh: None,
            profile_zsh: None,
            packages: None,
        };

        let expected_string = indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##               https://flox.dev/docs/concepts/manifest
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
            version = 1


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
            [install]
            # gum.pkg-path = "gum"
            # gum.version = "^0.14.5"


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
            [vars]
            # INTRO_MESSAGE = "It's gettin' Flox in here"


            ## Activation Hook ---------------------------------------------------
            ##  ... run by _bash_ shell when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [hook]
            on-activate = '''
              # Print something
              echo "hello world"

              # Set a environment variable
              $FOO="bar"
            '''


            ## Profile script ----------------------------------------------------
            ## ... sourced by _your shell_ when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [profile]
            # common = '''
            #   gum style \
            #   --foreground 212 --border-foreground 212 --border double \
            #   --align center --width 50 --margin "1 2" --padding "2 4" \
            #     $INTRO_MESSAGE
            # '''
            ## Shell specific profiles go here:
            # bash = ...
            # zsh  = ...
            # fish = ...


            ## Services ----------------------------------------------------------
            ##  $ flox services start             <- Starts all services
            ##  $ flox services status            <- Status of running services
            ##  $ flox activate --start-services  <- Activates & starts all
            ## -------------------------------------------------------------------
            [services]
            # myservice.command = "python3 -m http.server"


            ## Other Environment Options -----------------------------------------
            [options]
            # Systems that environment is compatible with
            systems = [
              "x86_64-linux",
            ]
            # Uncomment to disable CUDA detection.
            # cuda-detection = false
        "#};

        let manifest = RawManifest::new_documented(systems.as_slice(), &customization);
        assert_eq!(manifest.to_string(), expected_string.to_string());
    }

    #[test]
    fn create_documented_profile_script() {
        let systems = [&"x86_64-linux".to_string()];
        let customization = InitCustomization {
            hook_on_activate: None,
            profile_common: Some(
                indoc! { r#"
                    echo "Hello from Flox"
                "#}
                .to_string(),
            ),
            profile_bash: None,
            profile_fish: None,
            profile_tcsh: None,
            profile_zsh: None,
            packages: None,
        };

        let expected_string = indoc! {r#"
            ## Flox Environment Manifest -----------------------------------------
            ##
            ##   _Everything_ you need to know about the _manifest_ is here:
            ##
            ##               https://flox.dev/docs/concepts/manifest
            ##
            ## -------------------------------------------------------------------
            # Flox manifest version managed by Flox CLI
            version = 1


            ## Install Packages --------------------------------------------------
            ##  $ flox install gum  <- puts a package in [install] section below
            ##  $ flox search gum   <- search for a package
            ##  $ flox show gum     <- show all versions of a package
            ## -------------------------------------------------------------------
            [install]
            # gum.pkg-path = "gum"
            # gum.version = "^0.14.5"


            ## Environment Variables ---------------------------------------------
            ##  ... available for use in the activated environment
            ##      as well as [hook], [profile] scripts and [services] below.
            ## -------------------------------------------------------------------
            [vars]
            # INTRO_MESSAGE = "It's gettin' Flox in here"


            ## Activation Hook ---------------------------------------------------
            ##  ... run by _bash_ shell when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [hook]
            # on-activate = '''
            #   # -> Set variables, create files and directories
            #   # -> Perform initialization steps, e.g. create a python venv
            #   # -> Useful environment variables:
            #   #      - FLOX_ENV_PROJECT=/home/user/example
            #   #      - FLOX_ENV=/home/user/example/.flox/run
            #   #      - FLOX_ENV_CACHE=/home/user/example/.flox/cache
            # '''


            ## Profile script ----------------------------------------------------
            ## ... sourced by _your shell_ when you run 'flox activate'.
            ## -------------------------------------------------------------------
            [profile]
            common = '''
              echo "Hello from Flox"
            '''


            ## Services ----------------------------------------------------------
            ##  $ flox services start             <- Starts all services
            ##  $ flox services status            <- Status of running services
            ##  $ flox activate --start-services  <- Activates & starts all
            ## -------------------------------------------------------------------
            [services]
            # myservice.command = "python3 -m http.server"


            ## Other Environment Options -----------------------------------------
            [options]
            # Systems that environment is compatible with
            systems = [
              "x86_64-linux",
            ]
            # Uncomment to disable CUDA detection.
            # cuda-detection = false
        "#};

        let manifest = RawManifest::new_documented(systems.as_slice(), &customization);
        assert_eq!(manifest.to_string(), expected_string.to_string());
    }

    #[test]
    fn detect_catalog_manifest() {
        assert!(toml_edit::de::from_str::<Manifest>(CATALOG_MANIFEST).is_ok());
    }

    #[test]
    fn insert_adds_new_package() {
        let test_packages = vec![PackageToInstall::Catalog(
            CatalogPackage::from_str("python").unwrap(),
        )];
        let pre_addition_toml = DUMMY_MANIFEST.parse::<DocumentMut>().unwrap();
        assert!(!contains_package(&pre_addition_toml, test_packages[0].id()).unwrap());
        let insertion =
            insert_packages(DUMMY_MANIFEST, &test_packages).expect("couldn't add package");
        assert!(
            insertion.new_toml.is_some(),
            "manifest was changed by install"
        );
        assert!(contains_package(&insertion.new_toml.unwrap(), test_packages[0].id()).unwrap());
    }

    #[test]
    fn no_change_adding_existing_package() {
        let test_packages = vec![PackageToInstall::Catalog(
            CatalogPackage::from_str("hello").unwrap(),
        )];
        let pre_addition_toml = DUMMY_MANIFEST.parse::<DocumentMut>().unwrap();
        assert!(contains_package(&pre_addition_toml, test_packages[0].id()).unwrap());
        let insertion = insert_packages(DUMMY_MANIFEST, &test_packages).unwrap();
        assert!(
            insertion.new_toml.is_none(),
            "manifest shouldn't be changed installing existing package"
        );
        assert!(
            insertion.already_installed.values().all(|p| *p),
            "all of the packages should be listed as already installed"
        );
    }

    #[test]
    fn insert_adds_install_table_when_missing() {
        let test_packages = vec![PackageToInstall::Catalog(
            CatalogPackage::from_str("foo").unwrap(),
        )];
        let insertion = insert_packages(CATALOG_MANIFEST, &test_packages).unwrap();
        assert!(
            contains_package(&insertion.new_toml.clone().unwrap(), test_packages[0].id()).unwrap()
        );
        assert!(
            insertion.new_toml.is_some(),
            "manifest was changed by install"
        );
        assert!(
            !insertion.already_installed.values().all(|p| *p),
            "none of the packages should be listed as already installed"
        );
    }

    #[test]
    fn insert_error_when_manifest_malformed() {
        let test_packages = vec![PackageToInstall::Catalog(
            CatalogPackage::from_str("foo").unwrap(),
        )];
        let attempted_insertion = insert_packages(BAD_MANIFEST, &test_packages);
        assert!(matches!(
            attempted_insertion,
            Err(TomlEditError::ParseManifest(_))
        ))
    }

    #[test]
    fn remove_error_when_manifest_malformed() {
        let test_packages = vec!["hello".to_owned()];
        let attempted_removal = remove_packages(BAD_MANIFEST, &test_packages);
        assert!(matches!(
            attempted_removal,
            Err(TomlEditError::ParseManifest(_))
        ))
    }

    #[test]
    fn error_when_install_table_missing() {
        let test_packages = vec!["hello".to_owned()];
        let removal = remove_packages("version = 1", &test_packages);
        assert!(matches!(removal, Err(TomlEditError::PackageNotFound(_))));
    }

    #[test]
    fn removes_all_requested_packages() {
        let test_packages = vec!["hello".to_owned(), "ripgrep".to_owned()];
        let toml = remove_packages(DUMMY_MANIFEST, &test_packages).unwrap();
        assert!(!contains_package(&toml, "hello").unwrap());
        assert!(!contains_package(&toml, "ripgrep").unwrap());
    }

    #[test]
    fn error_when_removing_nonexistent_package() {
        let test_packages = vec![
            "hello".to_owned(),
            "DOES_NOT_EXIST".to_owned(),
            "nodePackages.@".to_owned(),
        ];
        let removal = remove_packages(DUMMY_MANIFEST, &test_packages);
        assert!(matches!(removal, Err(TomlEditError::PackageNotFound(_))));
    }

    #[test]
    fn inserts_package_needing_quotes() {
        let attrs = r#"foo."bar.baz".qux"#;
        let test_packages = vec![PackageToInstall::Catalog(
            CatalogPackage::from_str(attrs).unwrap(),
        )];
        let pre_addition_toml = DUMMY_MANIFEST.parse::<DocumentMut>().unwrap();
        assert!(!contains_package(&pre_addition_toml, test_packages[0].id()).unwrap());
        let insertion =
            insert_packages(DUMMY_MANIFEST, &test_packages).expect("couldn't add package");
        assert!(
            insertion.new_toml.is_some(),
            "manifest was changed by install"
        );
        let new_toml = insertion.new_toml.unwrap();
        assert!(contains_package(&new_toml, test_packages[0].id()).unwrap());
        let inserted_path = new_toml
            .get("install")
            .and_then(|t| t.get("qux"))
            .and_then(|d| d.get("pkg-path"))
            .and_then(|p| p.as_str())
            .unwrap();
        assert_eq!(inserted_path, r#"foo."bar.baz".qux"#);
    }

    #[test]
    fn parses_string_descriptor() {
        let parsed: CatalogPackage = "hello".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "hello".to_string(),
            pkg_path: "hello".to_string(),
            version: None,
            systems: None,
        });
        let parsed: CatalogPackage = "foo.bar@=1.2.3".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "bar".to_string(),
            pkg_path: "foo.bar".to_string(),
            version: Some("=1.2.3".to_string()),
            systems: None,
        });
        let parsed: CatalogPackage = "foo.bar@23.11".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "bar".to_string(),
            pkg_path: "foo.bar".to_string(),
            version: Some("23.11".to_string()),
            systems: None,
        });
        let parsed: CatalogPackage = "rubyPackages.\"http_parser.rb\"".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "\"http_parser.rb\"".to_string(),
            pkg_path: "rubyPackages.\"http_parser.rb\"".to_string(),
            version: None,
            systems: None,
        });

        // Attributes starting with `@` are allowed, the @ is not delimting the version if following a '.'
        let parsed: CatalogPackage = "nodePackages.@angular@1.2.3".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "@angular".to_string(),
            pkg_path: "nodePackages.@angular".to_string(),
            version: Some("1.2.3".to_string()),
            systems: None,
        });

        // Attributes starting with `@` are allowed, the @ is not delimting the version
        // if its the first character
        let parsed: CatalogPackage = "@1.2.3".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "3".to_string(),
            pkg_path: "@1.2.3".to_string(),
            version: None,
            systems: None,
        });

        // Attributes starting with `@` are allowed, the @ is not delimting the version
        // if its the first character.
        // Following `@` may delimit a version
        let parsed: CatalogPackage = "@pkg@version".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "@pkg".to_string(),
            pkg_path: "@pkg".to_string(),
            version: Some("version".to_string()),
            systems: None,
        });

        CatalogPackage::from_str("foo.\"bar.baz.qux@1.2.3")
            .expect_err("missing closing quote should cause failure");
        CatalogPackage::from_str("foo@").expect_err("missing version should cause failure");
    }

    proptest! {
        #[test]
        fn manifest_round_trip(manifest in any::<Manifest>()) {
            let toml = toml_edit::ser::to_string(&manifest).unwrap();
            let parsed = toml_edit::de::from_str::<Manifest>(&toml).unwrap();
            prop_assert_eq!(manifest, parsed);
        }
    }

    /// Determines whether to have a branch and/or revision in the URL
    #[derive(Debug, Arbitrary, PartialEq)]
    enum FlakeRefPathAttrs {
        None,
        RevPath,
        RevParam,
        RefParam,
        RefAndRevParams,
    }

    /// The components of an attrpath after `packages.<system>.`
    #[derive(Debug, Arbitrary, PartialEq)]
    enum AttrPathComponent {
        Bare,
        Quoted,
        QuotedWithDots,
    }

    /// The type of URL in the flake reference
    #[derive(Debug, Arbitrary, PartialEq)]
    enum FlakeRefURLType {
        GitHub,
        Https,
        GitHttps,
    }

    /// Flake ref outputs
    #[derive(Debug, Arbitrary, PartialEq)]
    enum FlakeRefOutputs {
        None,
        All,
        Out,
        OutAndMan,
    }

    #[derive(Debug, Arbitrary, PartialEq)]
    enum PkgFragment {
        None,
        Name(AttrPathComponent),
        #[proptest(
            strategy = "proptest::collection::vec(any::<AttrPathComponent>(), 1..=2).prop_map(PkgFragment::AttrPath)"
        )]
        AttrPath(Vec<AttrPathComponent>),
    }

    #[derive(Debug, Arbitrary)]
    struct ArbitraryFlakeRefURL {
        url_type: FlakeRefURLType,
        path_attrs: FlakeRefPathAttrs,
        pkg_fragment: PkgFragment,
        outputs: FlakeRefOutputs,
    }

    fn arbitrary_flake_ref_url() -> impl Strategy<Value = (String, String)> {
        any::<ArbitraryFlakeRefURL>()
            .prop_filter("don't add rev as path segment on arbitrary URLs", |seed| {
                (seed.url_type == FlakeRefURLType::Https)
                    && (seed.path_attrs != FlakeRefPathAttrs::RevPath)
            })
            .prop_map(|url_seed| {
                let stem = match url_seed.url_type {
                    FlakeRefURLType::GitHub => "github:foo/bar",
                    FlakeRefURLType::Https => "https://example.com/foo/bar",
                    FlakeRefURLType::GitHttps => "git+https://example.com/foo/bar",
                };
                let path_attrs = match url_seed.path_attrs {
                    FlakeRefPathAttrs::None => "",
                    FlakeRefPathAttrs::RevPath => "/abc123",
                    FlakeRefPathAttrs::RefParam => "?ref=master",
                    FlakeRefPathAttrs::RevParam => "?rev=abc123",
                    FlakeRefPathAttrs::RefAndRevParams => "?ref=master&rev=abc123",
                };
                let (fragment, expected_install_id) = match url_seed.pkg_fragment {
                    PkgFragment::None => {
                        if url_seed.outputs != FlakeRefOutputs::None {
                            ("#".to_string(), "bar")
                        } else {
                            (String::new(), "bar")
                        }
                    },
                    PkgFragment::Name(attr) => {
                        let id = match attr {
                            AttrPathComponent::Bare => "floxtastic",
                            AttrPathComponent::Quoted => "\"floxtastic\"",
                            AttrPathComponent::QuotedWithDots => "\"flox.tastic\"",
                        };
                        (format!("#{}", id), id)
                    },
                    PkgFragment::AttrPath(attr_path_seeds) => match attr_path_seeds.len() {
                        1 => {
                            let id = match attr_path_seeds[0] {
                                AttrPathComponent::Bare => "floxtastic",
                                AttrPathComponent::Quoted => "\"floxtastic\"",
                                AttrPathComponent::QuotedWithDots => "\"flox.tastic\"",
                            };
                            (format!("#legacyPackages.aarch64-darwin.{}", id), id)
                        },
                        2 => {
                            let namespace = match attr_path_seeds[0] {
                                AttrPathComponent::Bare => "nested",
                                AttrPathComponent::Quoted => "\"nested\"",
                                AttrPathComponent::QuotedWithDots => "\"nest.ed\"",
                            };
                            let id = match attr_path_seeds[1] {
                                AttrPathComponent::Bare => "floxtastic",
                                AttrPathComponent::Quoted => "\"floxtastic\"",
                                AttrPathComponent::QuotedWithDots => "\"flox.tastic\"",
                            };
                            (
                                format!("#legacyPackages.aarch64-darwin.{}.{}", namespace, id),
                                id,
                            )
                        },
                        _ => unreachable!(),
                    },
                };
                let outputs = match url_seed.outputs {
                    FlakeRefOutputs::None => "".to_string(),
                    FlakeRefOutputs::All => "^*".to_string(),
                    FlakeRefOutputs::Out => "^out".to_string(),
                    FlakeRefOutputs::OutAndMan => "^out,man".to_string(),
                };
                let url = format!("{}{}{}{}", stem, path_attrs, fragment, outputs);
                (url, expected_install_id.to_string())
            })
    }

    proptest! {
        #[test]
        fn infers_install_id_from_arbitrary_flake_ref_url((url, expected_id) in arbitrary_flake_ref_url()) {
            let url = Url::parse(&url).unwrap();
            let inferred = infer_flake_install_id(&url).unwrap();
            prop_assert_eq!(inferred, expected_id);
        }
    }

    #[test]
    fn infers_id_from_tarball_flake_ref() {
        // This is one case not covered by the proptest above
        let url = Url::parse("https://github.com/foo/bar/archive/main.tar.gz").unwrap();
        let inferred = infer_flake_install_id(&url).unwrap();
        assert_eq!(inferred.as_str(), "main.tar.gz");
    }

    fn assert_store_path_values(
        descriptor: &str,
        expected_path: &str,
        expected_id: &str,
        expected_system: &System,
    ) {
        let StorePath {
            system,
            store_path,
            id,
        } = StorePath::parse(expected_system, descriptor).expect("valid store path");
        assert_eq!(&system, expected_system);
        assert_eq!(&store_path, Path::new(expected_path));
        assert_eq!(id, expected_id);
    }

    #[test]
    fn parses_store_path() {
        let dummy_system = &"dummy-system".to_string();

        // invalid store paths
        StorePath::parse(dummy_system, "foo").expect_err("store path must be a full path");
        StorePath::parse(dummy_system, "/nix/store/foo")
            .expect_err("store path must contain a '-' separated hash");
        StorePath::parse(dummy_system, "/nicht/speicher/hash-foo")
            .expect_err("store path must be in /nix/store");

        // hash is stripped from the id
        assert_store_path_values(
            "/nix/store/hash-foo",
            "/nix/store/hash-foo",
            "foo",
            dummy_system,
        );

        // version is stripped in the id
        assert_store_path_values(
            "/nix/store/hash-apache-httpd-2.0.48",
            "/nix/store/hash-apache-httpd-2.0.48",
            "apache-httpd",
            dummy_system,
        );
        // non version fields are retained
        assert_store_path_values(
            "/nix/store/hash-foo-bar",
            "/nix/store/hash-foo-bar",
            "foo-bar",
            dummy_system,
        );

        // extra path components are ignored
        assert_store_path_values(
            "/nix/store/hash-apache-httpd-2.0.48/bin/httpd",
            "/nix/store/hash-apache-httpd-2.0.48",
            "apache-httpd",
            dummy_system,
        );
    }

    #[test]
    fn parses_build_section() {
        let build_manifest = indoc! {r#"
            version = 1
            [build]
            test.command = 'hello'

        "#};

        let parsed = toml_edit::de::from_str::<Manifest>(build_manifest).unwrap();

        assert_eq!(
            parsed.build,
            ManifestBuild(
                [("test".to_string(), ManifestBuildDescriptor {
                    command: "hello".to_string(),
                    runtime_packages: None,
                    files: None,
                    systems: None,
                    sandbox: None
                })]
                .into()
            )
        );
    }

    #[test]
    fn filter_services_by_system() {
        let manifest = indoc! {r#"
            version = 1
            [services]
            postgres.command = "postgres"
            mysql.command = "mysql"
            mysql.systems = ["x86_64-linux", "aarch64-linux"]
            redis.command = "redis"
            redis.systems = ["aarch64-linux"]
        "#};

        let parsed = toml_edit::de::from_str::<Manifest>(manifest).unwrap();

        assert_eq!(parsed.services.len(), 3, "{:?}", parsed.services);

        let filtered = parsed.services.copy_for_system(&"x86_64-linux".to_string());
        assert_eq!(filtered.len(), 2, "{:?}", filtered);
        assert!(filtered.contains_key("postgres"));
        assert!(filtered.contains_key("mysql"));

        let filtered = parsed
            .services
            .copy_for_system(&"aarch64-darwin".to_string());
        assert_eq!(filtered.len(), 1, "{:?}", filtered);
        assert!(filtered.contains_key("postgres"));
    }
}
