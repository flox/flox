use std::collections::{BTreeMap, HashMap};
use std::process::Command;
use std::str::FromStr;

use log::debug;
use serde::de::Error;
use serde::{Deserialize, Serialize};
use toml_edit::{self, DocumentMut, Formatted, InlineTable, Item, Table, Value};

use crate::data::{System, Version};
use crate::models::pkgdb::PKGDB_BIN;

pub(super) const DEFAULT_GROUP_NAME: &str = "toplevel";

/// A wrapper around a [`toml_edit::DocumentMut`]
/// that allows modifications of the raw manifest document,
/// while preserving comments and user formatting.
#[derive(Debug)]
struct RawManifest(toml_edit::DocumentMut);
impl RawManifest {
    /// Get the version of the manifest, if it's present or default to version 1.
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
    fn to_typed(&self) -> Result<TypedManifest, toml_edit::de::Error> {
        match self.get_version() {
            Some(1) => Ok(TypedManifest::Catalog(toml_edit::de::from_document(
                self.0.clone(),
            )?)),
            None => Ok(TypedManifest::Pkgdb(toml_edit::de::from_document(
                self.0.clone(),
            )?)),
            Some(v) => {
                let msg = format!("unsupported manifest version: {v}");
                Err(toml_edit::de::Error::custom(msg))
            },
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

/// Represents the Manifest data schema for reading/processing of the manifest.
/// Writing a [`TypedManifest`] will drop comments and formatting.
/// Hence, this should only be used in cases where these can safely be severed.
/// Edits to the user facing manifest.toml file should be made using [`RawManifest`] instead.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(untagged)]
enum TypedManifest {
    /// v2 manifest, processed by flox and resolved using the catalog service
    Catalog(Box<TypedManifestCatalog>),
    /// deprecated v1 manifest, processed entirely by `pkgdb`
    #[cfg_attr(test, proptest(skip))]
    Pkgdb(TypedManifestPkgdb),
}

/// Not meant for writing manifest files, only for reading them.
/// Modifications should be made using the the raw functions in this module.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(super) struct TypedManifestCatalog {
    version: Version<1>,
    /// The packages to install in the form of a map from package name
    /// to package descriptor.
    #[serde(default)]
    pub(super) install: ManifestInstall,
    /// Variables that are exported to the shell environment upon activation.
    #[serde(default)]
    vars: ManifestVariables,
    /// Hooks that are run at various times during the lifecycle of the manifest
    /// in a known shell environment.
    #[serde(default)]
    hook: ManifestHook,
    /// Profile scripts that are run in the user's shell upon activation.
    #[serde(default)]
    profile: ManifestProfile,
    /// Options that control the behavior of the manifest.
    #[serde(default)]
    options: ManifestOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, derive_more::Deref)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(super) struct ManifestInstall(BTreeMap<String, ManifestPackageDescriptor>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub(super) struct ManifestPackageDescriptor {
    pub(super) pkg_path: String,
    pub(super) package_group: Option<String>,
    pub(super) priority: Option<usize>,
    pub(super) version: Option<String>,
    pub(super) systems: Option<Vec<System>>,
    #[serde(default)]
    pub(super) optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(super) struct ManifestVariables(BTreeMap<String, String>);

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub(super) struct ManifestHook {
    /// A script that is run at activation time,
    /// in a flox provided bash shell
    on_activate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(super) struct ManifestProfile {
    /// When defined, this hook is run by _all_ shells upon activation
    common: Option<String>,
    /// When defined, this hook is run upon activation in a bash shell
    bash: Option<String>,
    /// When defined, this hook is run upon activation in a zsh shell
    zsh: Option<String>,
    /// When defined, this hook is run upon activation in a fish shell
    fish: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "kebab-case")]
pub(super) struct ManifestOptions {
    /// A list of systems that each package is resolved for.
    #[serde(default)]
    systems: Vec<System>,
    /// Options that control what types of packages are allowed.
    #[serde(default)]
    allows: Allows,
    /// Options that control how semver versions are resolved.
    #[serde(default)]
    semver: SemverOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(super) struct Allows {
    /// Whether to allow packages that are marked as `unfree`
    unfree: Option<bool>,
    /// Whether to allow packages that are marked as `broken`
    broken: Option<bool>,
    /// A list of license descriptors that are allowed
    #[serde(default)]
    licenses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub(super) struct SemverOptions {
    /// Whether to prefer pre-release versions when resolving
    #[serde(default)]
    prefer_pre_releases: Option<bool>,
}

/// Deserialize the manifest as a [serde_json::Value],
/// then convert it to a [RawManifest] that can then be converted to a [TypedManifest].
/// This provides more precise errors based on the version of the manifest.
///
/// See the comment on [`RawManifest::to_typed`] for more information.
impl<'de> Deserialize<'de> for TypedManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let document = toml_edit::ser::to_document(&value)
            .map_err(|err| serde::de::Error::custom(err.to_string()))?;
        RawManifest(document)
            .to_typed()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("couldn't parse descriptor '{}': {}", desc, msg)]
    MalformedStringDescriptor { msg: String, desc: String },
    /// FIXME: This is a temporary error variant until `flox` parses descriptors on its own
    #[error("failed while calling pkgdb")]
    PkgDbCall(#[source] std::io::Error),
}

/// A subset of the manifest used to check what type of edits users make. We
/// don't use this struct for making our own edits.
///
/// The authoritative form of the manifest is in
/// https://github.com/flox/pkgdb/blob/main/include/flox/resolver/manifest-raw.hh#L263
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct TypedManifestPkgdb {
    pub vars: Option<toml::Table>,
    pub hook: Option<toml::Table>,
    #[serde(flatten)]
    _toml: toml::Table,
}

/// An alias to the Pkgdb backed Maifest Schema for backwards compatibility.
/// TODO: remove this as part of <https://github.com/flox/flox/issues/1320>
pub type Manifest = TypedManifestPkgdb;

/// An error encountered while installing packages.
#[derive(Debug, thiserror::Error)]
pub enum TomlEditError {
    /// The provided string couldn't be parsed into a valid TOML document
    #[error("couldn't parse manifest contents: {0}")]
    ParseManifest(toml_edit::TomlError),
    /// The provided string was a valid TOML file, but it didn't have
    /// the format that we anticipated.
    #[error("'install' must be a table, but found {0} instead")]
    MalformedInstallTable(String),
    /// The `[install]` table was missing entirely
    #[error("'install' table not found")]
    MissingInstallTable,
    /// Tried to uninstall a package that wasn't installed
    #[error("couldn't uninstall '{0}', wasn't previously installed")]
    PackageNotFound(String),
    #[error("'options' must be a table, but found {0} instead")]
    MalformedOptionsTable(String),
    #[error("'options' must be an array, but found {0} instead")]
    MalformedOptionsSystemsArray(String),
}

/// Records the result of trying to install a collection of packages to the
#[derive(Debug)]
pub struct PackageInsertion {
    pub new_toml: Option<DocumentMut>,
    pub already_installed: HashMap<String, bool>,
}

/// A package to install.
///
/// Users may specify a different install ID than the package name,
/// especially when the package is nested. This struct is the common
/// denominator for packages with specified IDs and packages with
/// default IDs.
#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageToInstall {
    pub id: String,
    pub pkg_path: String,
    pub version: Option<String>,
    pub input: Option<String>,
}

impl FromStr for PackageToInstall {
    type Err = ManifestError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        temporary_parse_descriptor(s)
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
    let mut toml = manifest_contents
        .parse::<DocumentMut>()
        .map_err(TomlEditError::ParseManifest)?;

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
        if !install_table.contains_key(&pkg.id) {
            let mut descriptor_table = InlineTable::new();
            descriptor_table.insert(
                "pkg-path",
                Value::String(Formatted::new(pkg.pkg_path.clone())),
            );
            if let Some(ref version) = pkg.version {
                descriptor_table.insert("version", Value::String(Formatted::new(version.clone())));
            }
            if let Some(ref input) = pkg.input {
                descriptor_table.insert("input", Value::String(Formatted::new(input.clone())));
            }
            descriptor_table.set_dotted(true);
            install_table.insert(&pkg.id, Item::Value(Value::InlineTable(descriptor_table)));
            already_installed.insert(pkg.id.clone(), false);
            debug!(
                "package newly installed: id={}, pkg-path={}",
                pkg.id, pkg.pkg_path
            );
        } else {
            already_installed.insert(pkg.id.clone(), true);
            debug!("package already installed: id={}", pkg.id);
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

/// Remove package names from the `[install]` table of a manifest
pub fn remove_packages(
    manifest_contents: &str,
    pkgs: &[String],
) -> Result<DocumentMut, TomlEditError> {
    debug!("attempting to remove packages from the manifest");
    let mut toml = manifest_contents
        .parse::<DocumentMut>()
        .map_err(TomlEditError::ParseManifest)?;

    let installs_table = {
        let installs_field = toml
            .get_mut("install")
            .ok_or(TomlEditError::PackageNotFound(pkgs[0].clone()))?;

        let type_name = installs_field.type_name().into();

        installs_field
            .as_table_mut()
            .ok_or(TomlEditError::MalformedInstallTable(type_name))?
    };

    for pkg in pkgs {
        debug!("checking for presence of package '{pkg}'");
        if !installs_table.contains_key(pkg) {
            debug!("package '{pkg}' wasn't found");
            return Err(TomlEditError::PackageNotFound(pkg.clone()));
        } else {
            installs_table.remove(pkg);
            debug!("package '{pkg}' was removed");
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
        .parse::<DocumentMut>()
        .map_err(TomlEditError::ParseManifest)?;

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

/// A parsed descriptor from `pkgdb parse descriptor --manifest`
///
/// FIXME: this is currently a hack using a tool in `pkgdb` only meant for debugging.
#[derive(Debug, Deserialize)]
pub struct ParsedDescriptor {
    pub name: Option<String>,
    #[serde(rename = "pkg-path")]
    pub pkg_path: Option<Vec<String>>,
    pub input: Option<Input>,
    pub version: Option<String>,
    pub semver: Option<String>,
}

/// A parsed input
///
/// FIXME: this is currently a hack using a tool in `pkgdb` only meant for debugging.
#[derive(Debug, Deserialize, PartialEq)]
pub struct Input {
    pub id: String,
}

/// Parse a shorthand descriptor into structured data
///
/// FIXME: this is currently a hack using a tool in `pkgdb` only meant for debugging.
pub fn temporary_parse_descriptor(descriptor: &str) -> Result<PackageToInstall, ManifestError> {
    let output = Command::new(&*PKGDB_BIN)
        .arg("parse")
        .arg("descriptor")
        .arg("--to")
        .arg("manifest")
        .arg(descriptor)
        .output()
        .map_err(ManifestError::PkgDbCall)?;
    let parsed: Result<ParsedDescriptor, _> = serde_json::from_slice(&output.stdout);
    if let Ok(parsed) = parsed {
        let (id, path) = if let Some(mut path) = parsed.pkg_path {
            // Quote any path components that need quoting
            path.iter_mut().for_each(|attr| {
                if attr.contains('.') || attr.contains('"') {
                    *attr = format!("\"{}\"", attr);
                }
            });
            let id_part = path.last().cloned().map(Ok).unwrap_or_else(|| {
                Err(ManifestError::MalformedStringDescriptor {
                    msg: "descriptor had an empty path".to_string(),
                    desc: descriptor.to_string(),
                })
            })?;
            (id_part, path.join("."))
        } else if let Some(name) = parsed.name {
            (name.clone(), name)
        } else {
            // This should have been caught by `pkgdb parse descriptor`,
            // it's more likely that if we hit this we got unexpected output
            // that we didn't know how to parse. It's possible, but unlikely,
            // and something ignored for the sake of time when we can write
            // a parser in `flox` instead of leaning on `pkgdb` debugging tools.
            return Err(ManifestError::MalformedStringDescriptor {
                msg: "descriptor had no name or path".to_string(),
                desc: descriptor.to_string(),
            });
        };
        let version = if let Some(version) = parsed.version {
            let mut v = "=".to_string();
            v.push_str(&version);
            Some(v)
        } else {
            parsed.semver
        };
        let input = parsed.input.map(|input| input.id);
        Ok(PackageToInstall {
            id,
            pkg_path: path,
            version,
            input,
        })
    } else {
        Err(ManifestError::MalformedStringDescriptor {
            msg: String::from_utf8_lossy(&output.stdout).to_string(),
            desc: descriptor.to_string(),
        })
    }
}

#[cfg(test)]
mod test {
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    const DUMMY_MANIFEST: &str = r#"
[install]
hello = {}

[install.ripgrep]
[install.bat]
        "#;

    // This is an array of tables called `install` rather than a table called `install`.
    const BAD_MANIFEST: &str = r#"
[[install]]
python = {}

[[install]]
ripgrep = {}
        "#;

    #[test]
    fn detect_pkgdb_manifest() {
        const PKGDB_MANIFEST: &str = indoc! {r#"
            # No version field, so it's a pkgdb manifest
        "#};

        assert!(matches!(
            toml_edit::de::from_str(PKGDB_MANIFEST),
            Ok(TypedManifest::Pkgdb(_))
        ))
    }

    #[test]
    fn detect_catalog_manifest() {
        const CATALOG_MANIFEST: &str = indoc! {r#"
            version = 1
        "#};

        assert!(matches!(
            toml_edit::de::from_str(CATALOG_MANIFEST),
            Ok(TypedManifest::Catalog(_))
        ))
    }

    #[test]
    fn insert_adds_new_package() {
        let test_packages = vec![PackageToInstall::from_str("python").unwrap()];
        let pre_addition_toml = DUMMY_MANIFEST.parse::<DocumentMut>().unwrap();
        assert!(!contains_package(&pre_addition_toml, &test_packages[0].id).unwrap());
        let insertion =
            insert_packages(DUMMY_MANIFEST, &test_packages).expect("couldn't add package");
        assert!(
            insertion.new_toml.is_some(),
            "manifest was changed by install"
        );
        assert!(contains_package(&insertion.new_toml.unwrap(), &test_packages[0].id).unwrap());
    }

    #[test]
    fn no_change_adding_existing_package() {
        let test_packages = vec![PackageToInstall::from_str("hello").unwrap()];
        let pre_addition_toml = DUMMY_MANIFEST.parse::<DocumentMut>().unwrap();
        assert!(contains_package(&pre_addition_toml, &test_packages[0].id).unwrap());
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
        let test_packages = vec![PackageToInstall::from_str("foo").unwrap()];
        let insertion = insert_packages("", &test_packages).unwrap();
        assert!(
            contains_package(&insertion.new_toml.clone().unwrap(), &test_packages[0].id).unwrap()
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
        let test_packages = vec![PackageToInstall::from_str("foo").unwrap()];
        let attempted_insertion = insert_packages(BAD_MANIFEST, &test_packages);
        assert!(matches!(
            attempted_insertion,
            Err(TomlEditError::MalformedInstallTable(_))
        ))
    }

    #[test]
    fn remove_error_when_manifest_malformed() {
        let test_packages = vec!["hello".to_owned()];
        let attempted_removal = remove_packages(BAD_MANIFEST, &test_packages);
        assert!(matches!(
            attempted_removal,
            Err(TomlEditError::MalformedInstallTable(_))
        ))
    }

    #[test]
    fn error_when_install_table_missing() {
        let test_packages = vec!["hello".to_owned()];
        let removal = remove_packages("", &test_packages);
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
        let test_packages = vec!["hello".to_owned(), "DOES_NOT_EXIST".to_owned()];
        let removal = remove_packages(DUMMY_MANIFEST, &test_packages);
        assert!(matches!(removal, Err(TomlEditError::PackageNotFound(_))));
    }

    #[test]
    fn inserts_package_needing_quotes() {
        let attrs = r#"foo."bar.baz".qux"#;
        let test_packages = vec![PackageToInstall::from_str(attrs).unwrap()];
        let pre_addition_toml = DUMMY_MANIFEST.parse::<DocumentMut>().unwrap();
        assert!(!contains_package(&pre_addition_toml, &test_packages[0].id).unwrap());
        let insertion =
            insert_packages(DUMMY_MANIFEST, &test_packages).expect("couldn't add package");
        assert!(
            insertion.new_toml.is_some(),
            "manifest was changed by install"
        );
        let new_toml = insertion.new_toml.unwrap();
        assert!(contains_package(&new_toml, &test_packages[0].id).unwrap());
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
        // FIXME: remove or update this test when `flox` can parse descriptors on its own
        let parsed = temporary_parse_descriptor("hello").unwrap();
        assert_eq!(parsed, PackageToInstall {
            id: "hello".to_string(),
            pkg_path: "hello".to_string(),
            version: None,
            input: None,
        });
        let parsed = temporary_parse_descriptor("nixpkgs:foo.bar@=1.2.3").unwrap();
        assert_eq!(parsed, PackageToInstall {
            id: "bar".to_string(),
            pkg_path: "foo.bar".to_string(),
            version: Some("=1.2.3".to_string()),
            input: Some("nixpkgs".to_string())
        });
        let parsed = temporary_parse_descriptor("nixpkgs:foo.bar@23.11").unwrap();
        assert_eq!(parsed, PackageToInstall {
            id: "bar".to_string(),
            pkg_path: "foo.bar".to_string(),
            version: Some("23.11".to_string()),
            input: Some("nixpkgs".to_string())
        });
        let parsed = temporary_parse_descriptor("nixpkgs:rubyPackages.\"http_parser.rb\"").unwrap();
        assert_eq!(parsed, PackageToInstall {
            id: "\"http_parser.rb\"".to_string(),
            pkg_path: "rubyPackages.\"http_parser.rb\"".to_string(),
            version: None,
            input: Some("nixpkgs".to_string())
        });
    }
}
