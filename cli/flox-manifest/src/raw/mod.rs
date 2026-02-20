use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::LazyLock;

use flox_core::activate::mode::ActivateMode;
use flox_core::data::System;
use indoc::indoc;
use itertools::Itertools;
use reqwest::Url;
use toml_edit::{self, Array, DocumentMut, Formatted, InlineTable, Item, Table, TableLike, Value};
use tracing::{debug, trace};

use crate::interfaces::CommonFields;
use crate::parsed::common::{self, KnownSchemaVersion, VersionKind};
use crate::parsed::v1_10_0::SelectedOutputs;
use crate::parsed::{Inner, v1, v1_10_0};
use crate::util::is_custom_package;
use crate::{Manifest, ManifestError, Migrated, Parsed, Validated};

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
/// Represents the `[include]` table key in manifest.toml
pub const MANIFEST_INCLUDE_KEY: &str = "include";
/// Represents the `[build]` table key in manifest.toml
pub const MANIFEST_BUILD_KEY: &str = "build";

pub static DEFAULT_SYSTEMS_STR: LazyLock<[String; 4]> = LazyLock::new(|| {
    [
        "aarch64-darwin".into(),
        "aarch64-linux".into(),
        "x86_64-darwin".into(),
        "x86_64-linux".into(),
    ]
});

pub(crate) fn get_schema_version_kind(toml: &DocumentMut) -> Result<VersionKind, ManifestError> {
    if let Some(item) = toml.get("version") {
        if let Some(int) = item.as_integer() {
            Ok(VersionKind::Version(int as u8))
        } else {
            Err(ManifestError::Other(
                "'version' field must be an integer".into(),
            ))
        }
    } else if let Some(item) = toml.get("schema-version") {
        if let Some(s) = item.as_str() {
            Ok(VersionKind::SchemaVersion(s.to_string()))
        } else {
            Err(ManifestError::Other(
                "'schema-version' field must be a version string like \"X.Y.Z\"".into(),
            ))
        }
    } else {
        Err(ManifestError::MissingSchemaVersion)
    }
}

pub(crate) fn get_json_schema_version_kind(
    json: &serde_json::Value,
) -> Result<VersionKind, ManifestError> {
    if let Some(value) = json.get("version") {
        if let Some(int) = value.as_i64() {
            Ok(VersionKind::Version(int as u8))
        } else {
            Err(ManifestError::Other(
                "'version' field must be an integer".into(),
            ))
        }
    } else if let Some(item) = json.get("schema-version") {
        if let Some(s) = item.as_str() {
            Ok(VersionKind::SchemaVersion(s.to_string()))
        } else {
            Err(ManifestError::Other(
                "'schema-version' field must be a version string like \"X.Y.Z\"".into(),
            ))
        }
    } else {
        Err(ManifestError::MissingSchemaVersion)
    }
}

#[cfg(test)]
mod schema_version_tests {
    use super::*;

    fn parse_toml(s: impl AsRef<str>) -> DocumentMut {
        s.as_ref().parse::<DocumentMut>().unwrap()
    }

    #[test]
    fn missing_schema() {
        let toml = parse_toml("foo = 1");
        let err = get_schema_version_kind(&toml).err().unwrap();
        assert!(matches!(err, ManifestError::MissingSchemaVersion));
    }

    #[test]
    fn version_wrong_type() {
        let toml = parse_toml("version = true");
        let err = get_schema_version_kind(&toml).err().unwrap();
        assert!(matches!(err, ManifestError::Other(_)));
    }

    #[test]
    fn version() {
        let toml = parse_toml("version = 42");
        let VersionKind::Version(value) = get_schema_version_kind(&toml).unwrap() else {
            panic!()
        };
        assert_eq!(value, 42);
    }

    #[test]
    fn schema_version_wrong_type() {
        let toml = parse_toml("schema-version = 42");
        let err = get_schema_version_kind(&toml).err().unwrap();
        assert!(matches!(err, ManifestError::Other(_)));
    }

    #[test]
    fn schema_version() {
        let toml = parse_toml("schema-version = \"1.10.0\"");
        let VersionKind::SchemaVersion(value) = get_schema_version_kind(&toml).unwrap() else {
            panic!()
        };
        assert_eq!(value, "1.10.0".to_string());
    }
}

/// A profile script or list of packages to install when initializing an environment
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct InitCustomization {
    pub hook_on_activate: Option<String>,
    pub profile_common: Option<String>,
    pub profile_bash: Option<String>,
    pub profile_fish: Option<String>,
    pub profile_tcsh: Option<String>,
    pub profile_zsh: Option<String>,
    pub packages: Option<Vec<CatalogPackage>>,
    pub activate_mode: Option<ActivateMode>,
}

#[derive(Debug, thiserror::Error)]
pub enum RawManifestError {
    #[error("couldn't parse descriptor '{}': {}", desc, msg)]
    MalformedStringDescriptor { msg: String, desc: String },
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
    #[error("couldn't parse manifest contents: {0}")]
    ParseToml(toml_edit::TomlError),
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
    pub new_manifest: Option<Manifest<Migrated>>,
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
    pub fn parse(system: &System, s: &str) -> Result<Self, RawManifestError> {
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

    pub fn systems(&self) -> Option<Vec<String>> {
        match self {
            PackageToInstall::Catalog(pkg) => pkg.systems.clone(),
            PackageToInstall::Flake(_) => None,
            PackageToInstall::StorePath(pkg) => Some(vec![pkg.system.clone()]),
        }
    }
}

/// Tries to infer an install id from the flake ref URL, or falls back to "flake".
fn infer_flake_install_id(url: &Url) -> Result<String, RawManifestError> {
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
            .ok_or(RawManifestError::InvalidFlakeRef(url.to_string()))
    } else {
        url.path()
            .split('/')
            .next_back()
            .map(|s| url_escape::decode(s).to_string())
            .ok_or(RawManifestError::InvalidFlakeRef(url.to_string()))
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

/// Represents the outputs to install for a package.
/// This is the raw representation used in parsing CLI arguments.
#[derive(Debug, Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RawSelectedOutputs {
    /// Install all outputs (specified as `^..`)
    All,
    /// Install specific outputs (specified as `^out,man,dev`)
    Specific(Vec<String>),
}

impl RawSelectedOutputs {
    /// Parse outputs from a string (e.g., "..", "out,man,dev")
    pub fn parse(s: &str) -> Self {
        if s == ".." {
            Self::All
        } else {
            Self::Specific(s.split(',').map(|s| s.trim().to_string()).collect())
        }
    }
}

impl From<RawSelectedOutputs> for SelectedOutputs {
    fn from(value: RawSelectedOutputs) -> Self {
        match value {
            RawSelectedOutputs::All => SelectedOutputs::all(),
            RawSelectedOutputs::Specific(items) => SelectedOutputs::Specific(items),
        }
    }
}

impl From<&RawSelectedOutputs> for SelectedOutputs {
    fn from(value: &RawSelectedOutputs) -> Self {
        match value {
            RawSelectedOutputs::All => SelectedOutputs::all(),
            RawSelectedOutputs::Specific(items) => SelectedOutputs::Specific(items.clone()),
        }
    }
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
    /// Outputs to install for this package.
    /// If `None`, the default outputs are installed.
    /// This can be parsed from the shorthand descriptor using the `^` syntax.
    pub outputs: Option<RawSelectedOutputs>,
}

impl CatalogPackage {
    /// Returns true if the package is from a custom catalog.
    pub fn is_custom_catalog(&self) -> bool {
        is_custom_package(&self.pkg_path)
    }
}

impl FromStr for CatalogPackage {
    type Err = RawManifestError;

    /// Parse a shorthand descriptor into `install_id`, `attribute_path` and `version`.
    ///
    /// A shorthand descriptor consists of a package name and an optional version.
    /// The attribute path is a dot-separated path to a package in the catalog.
    /// The last component of the attribute path is the `install_id`.
    ///
    /// The descriptor is parsed as follows:
    /// ```text
    ///     descriptor ::= <attribute_path>[@<version>] | <attribute_path>[^<outputs>]
    ///
    ///     attribute_path ::= <install_id> | <attribute_path_rest>.<install_id>
    ///     attribute_path_rest ::= <identifier> | <attribute_path_rest>.<identifier>
    ///     install_id ::= <identifier> | @<identifier>
    ///
    ///     version ::= <string> # interpreted as semver or plain version by the resolver
    ///     outputs ::= ".." | <output_list> # interpreted as all outputs or specific outputs to install
    /// ```
    fn from_str(descriptor: &str) -> Result<Self, RawManifestError> {
        fn split_outputs(
            raw_str: &str,
        ) -> Result<(&str, Option<RawSelectedOutputs>), RawManifestError> {
            match raw_str.split_once('^') {
                Some((attr_path, outputs_str)) => {
                    if outputs_str.is_empty() {
                        return Err(RawManifestError::MalformedStringDescriptor {
                            msg: "expected output specification after '^'".to_string(),
                            desc: raw_str.to_string(),
                        });
                    }
                    let outputs = RawSelectedOutputs::parse(outputs_str);
                    Ok((attr_path, Some(outputs)))
                },
                None => Ok((raw_str, None)),
            }
        }
        fn split_version(haystack: &str) -> Result<(&str, Option<String>), RawManifestError> {
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
            let attr_path = &haystack[..version_at.unwrap_or(haystack.len())];
            let version = if let Some(version) = version {
                if version.is_empty() {
                    return Err(RawManifestError::MalformedStringDescriptor {
                        msg: indoc! {"
                        Expected version requirement after '@'.
                        Try adding quotes around the argument."}
                        .to_string(),
                        desc: haystack.to_string(),
                    });
                }
                Some(version.to_string())
            } else {
                None
            };
            Ok((attr_path, version))
        }

        let mut attr_path = descriptor;
        let mut outputs = None;
        let mut version = None;
        if attr_path.contains('@') {
            (attr_path, version) = split_version(descriptor)?;
        } else {
            (attr_path, outputs) = split_outputs(descriptor)?;
        }

        let install_id = install_id_from_attr_path(attr_path, descriptor)?;

        Ok(Self {
            id: install_id,
            pkg_path: attr_path.to_string(),
            version,
            systems: None,
            outputs,
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
    fn parse(system: &System, descriptor: &str) -> Result<Self, RawManifestError> {
        // Don't canonicalize the path if it's already a store path.
        // Canonicalizing a store path can potentially resolve it to a different path,
        // if the original path is a symlink to another store path.
        let path = if Path::new(descriptor).starts_with("/nix/store") {
            PathBuf::from(descriptor)
        } else {
            Path::new(descriptor).canonicalize().map_err(|e| {
                RawManifestError::MalformedStringDescriptor {
                    msg: format!("cannot resolve path: {}", e),
                    desc: descriptor.to_string(),
                }
            })?
        };

        // [sic] 4 components, because the root dir is counted as a component on its own
        let store_path: PathBuf = path.components().take(4).collect();
        let Ok(hash_and_name) = store_path.strip_prefix("/nix/store") else {
            return Err(RawManifestError::MalformedStringDescriptor {
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
            return Err(RawManifestError::MalformedStringDescriptor {
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

/// Infers an install ID from the last component of a slash or dot separated
/// attribute path, so that we get a user-friendly name without any catalog or
/// package hierachy.
/// Components within quotes are treated as a single component.
fn install_id_from_attr_path(
    attr_path: &str,
    descriptor: &str,
) -> Result<String, RawManifestError> {
    let mut install_id = None;
    let mut cur = String::new();

    let mut start_quote = None;

    for (n, c) in attr_path.chars().enumerate() {
        match c {
            '.' | '/' if start_quote.is_none() => {
                let _ = install_id.insert(std::mem::take(&mut cur));
            },
            '"' if start_quote.is_some() => {
                start_quote = None;
                cur.push('"');
            },
            '"' if start_quote.is_none() => {
                start_quote = Some(n);
                cur.push('"');
            },
            other => cur.push(other),
        }
    }

    if start_quote.is_some() {
        return Err(RawManifestError::MalformedStringDescriptor {
            msg: "unclosed quote".to_string(),
            desc: descriptor.to_string(),
        });
    }

    if !cur.is_empty() {
        let _ = install_id.insert(cur);
    }

    install_id.ok_or(RawManifestError::MalformedStringDescriptor {
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
        if let Some(ref outputs) = val.outputs {
            match outputs {
                RawSelectedOutputs::All => {
                    table.insert("outputs", Value::String(Formatted::new("all".to_string())));
                },
                RawSelectedOutputs::Specific(output_names) => {
                    table.insert(
                        "outputs",
                        Value::Array(
                            output_names
                                .iter()
                                .map(|s| Value::String(Formatted::new(s.clone())))
                                .collect(),
                        ),
                    );
                },
            }
        }
        table
    }
}

pub trait ModifyPackages {
    /// Adds new packages to the manifest, returning a new manifest if one was created
    /// along with a map of which of the newly added packages were already installed.
    fn add_packages(&self, pkgs: &[PackageToInstall]) -> Result<PackageInsertion, ManifestError>;
    fn remove_packages(&self, install_ids: &[String]) -> Result<Manifest<Migrated>, ManifestError>;
}

impl ModifyPackages for Manifest<Migrated> {
    fn add_packages(&self, pkgs: &[PackageToInstall]) -> Result<PackageInsertion, ManifestError> {
        let mut manifest = self.clone();
        let pkg_map = manifest.inner.migrated_parsed.install.inner_mut();
        let mut already_installed: HashMap<String, bool> = HashMap::new();
        for pkg in pkgs.iter() {
            if pkg_map.contains_key(pkg.id()) {
                already_installed.insert(pkg.id().to_string(), true);
                debug!("package already installed: id={}", pkg.id());
                continue;
            }
            already_installed.insert(pkg.id().to_string(), false);
            match pkg {
                PackageToInstall::Catalog(pkg_raw) => {
                    let pkg_group = if pkg_raw.is_custom_catalog() {
                        Some(pkg.id().to_string())
                    } else {
                        None
                    };
                    let catalog_descriptor = v1_10_0::PackageDescriptorCatalog {
                        pkg_path: pkg_raw.pkg_path.clone(),
                        pkg_group,
                        priority: None,
                        version: pkg_raw.version.clone(),
                        systems: pkg_raw.systems.clone(),
                        outputs: pkg_raw.outputs.clone().map(|outputs| outputs.into()),
                    };
                    let descriptor =
                        v1_10_0::ManifestPackageDescriptor::Catalog(catalog_descriptor);
                    pkg_map.insert(pkg.id().to_string(), descriptor);
                    debug!(
                        "package newly installed: id={}, pkg-path={}",
                        pkg_raw.id, pkg_raw.pkg_path
                    );
                },
                PackageToInstall::Flake(flake_raw) => {
                    let flake_descriptor = v1_10_0::PackageDescriptorFlake {
                        flake: flake_raw.url.to_string(),
                        priority: None,
                        systems: pkg.systems(),
                        outputs: None,
                    };
                    let descriptor = v1_10_0::ManifestPackageDescriptor::FlakeRef(flake_descriptor);
                    pkg_map.insert(pkg.id().to_string(), descriptor);
                    debug!(
                        "package newly installed: id={}, flakeref={}",
                        flake_raw.id,
                        flake_raw.url.to_string()
                    );
                },
                PackageToInstall::StorePath(store_path_raw) => {
                    let store_path_descriptor = common::PackageDescriptorStorePath {
                        store_path: store_path_raw.store_path.to_string_lossy().to_string(),
                        systems: None,
                        priority: None,
                    };
                    let descriptor =
                        v1_10_0::ManifestPackageDescriptor::StorePath(store_path_descriptor);
                    pkg_map.insert(pkg.id().to_string(), descriptor);
                    debug!(id=pkg.id(), store_path=%store_path_raw.store_path.display(),
                        "store path newly installed"
                    );
                },
            }
        }
        let new_manifest = if already_installed.values().all(|p| *p) {
            None
        } else {
            manifest.update_raw_packages_from_typed_manifest()?;
            Some(manifest)
        };
        Ok(PackageInsertion {
            new_manifest,
            already_installed,
        })
    }

    fn remove_packages(&self, install_ids: &[String]) -> Result<Manifest<Migrated>, ManifestError> {
        debug!("attempting to remove packages from the manifest");
        let mut manifest = self.clone();
        let pkg_map = manifest.inner.migrated_parsed.install.inner_mut();
        for install_id in install_ids.iter() {
            if !pkg_map.contains_key(install_id) {
                debug!(id = install_id, "package not found");
                return Err(ManifestError::PackageNotFound(install_id.clone()));
            }
            pkg_map.remove(install_id);
            debug!(id = install_id, "package removed");
        }
        manifest.update_raw_packages_from_typed_manifest()?;
        Ok(manifest)
    }
}

pub trait SyncTypedToRaw {
    /// Updates the TOML manifest to match the contents of the typed manifest.
    fn update_toml(&mut self) -> Result<(), ManifestError> {
        self.update_schema_version();
        self.update_systems()?;
        self.update_raw_packages_from_typed_manifest()?;
        Ok(())
    }

    /// Updates `options.systems` in the TOML manifest to match that in the
    /// typed manifest.
    fn update_systems(&mut self) -> Result<(), ManifestError>;

    /// Sets the `schema-version` or `version` field in the TOML manifest to match that in
    /// the typed manifest.
    ///
    /// An existing `version` key is removed when setting `schema-version` and vice versa.
    fn update_schema_version(&mut self);

    /// Updates the TOML manifest to only contain the package descriptors contained in the
    /// typed manifest, and updates their contents to match as well.
    fn update_raw_packages_from_typed_manifest(&mut self) -> Result<(), ManifestError>;
}

impl SyncTypedToRaw for Manifest<Validated> {
    fn update_schema_version(&mut self) {
        update_schema_version(&mut self.inner.raw, self.inner.parsed.schema_version());
    }

    fn update_systems(&mut self) -> Result<(), ManifestError> {
        update_systems(
            &mut self.inner.raw,
            self.inner.parsed.options().systems.as_ref(),
        )
        .map_err(ManifestError::TomlEdit)
    }

    fn update_raw_packages_from_typed_manifest(&mut self) -> Result<(), ManifestError> {
        update_raw_packages_from_typed_manifest(&mut self.inner.raw, &self.inner.parsed)
    }
}

impl SyncTypedToRaw for Manifest<Migrated> {
    fn update_schema_version(&mut self) {
        update_schema_version(&mut self.inner.migrated_raw, KnownSchemaVersion::latest());
    }

    fn update_systems(&mut self) -> Result<(), ManifestError> {
        update_systems(
            &mut self.inner.migrated_raw,
            self.inner.migrated_parsed.options().systems.as_ref(),
        )
        .map_err(ManifestError::TomlEdit)
    }

    fn update_raw_packages_from_typed_manifest(&mut self) -> Result<(), ManifestError> {
        update_raw_packages_from_typed_manifest(
            &mut self.inner.migrated_raw,
            &Parsed::from_latest(self.inner.migrated_parsed.clone()),
        )
    }
}

fn update_schema_version(raw: &mut DocumentMut, schema_version: KnownSchemaVersion) {
    match schema_version {
        KnownSchemaVersion::V1 => {
            if raw.get("schema-version").is_some() {
                raw.remove("schema-version");
            }
            raw.insert("version", toml_string(schema_version.to_string()).into());
        },
        KnownSchemaVersion::V1_10_0 => {
            if raw.get("version").is_some() {
                raw.remove("version");
            }
            raw.insert(
                "schema-version",
                toml_string(schema_version.to_string()).into(),
            );
        },
    }
}

fn update_systems(
    raw: &mut DocumentMut,
    maybe_systems: Option<&Vec<String>>,
) -> Result<(), TomlEditError> {
    // You need to check whether you actually need to touch the systems first,
    // otherwise you'll unconditionally create the `[options]` table.
    if let Some(systems) = maybe_systems {
        let options_field = raw
            .entry("options")
            .or_insert_with(|| Item::Table(Table::new()));
        let options_field_type = options_field.type_name().into();
        let options_table = options_field.as_table_mut().ok_or_else(|| {
            debug!("creating new [options] table");
            TomlEditError::MalformedOptionsTable(options_field_type)
        })?;
        options_table.insert("systems", toml_array_of_strings(systems).into());
    } else if let Some(options_field) = raw.get_mut("options") {
        let options_field_type = options_field.type_name().into();
        let options_table = options_field
            .as_table_mut()
            .ok_or(TomlEditError::MalformedOptionsTable(options_field_type))?;
        options_table.remove("systems");
    }
    Ok(())
}

/// Brings all package descriptors in a raw TOML manifest into sync with the typed package descriptors
/// in a validated manifest.
fn update_raw_packages_from_typed_manifest(
    raw: &mut DocumentMut,
    parsed: &Parsed,
) -> Result<(), ManifestError> {
    let install_table = get_install_table_mut(raw)?;
    let raw_pkgs = install_table
        .iter()
        .map(|(key, _value)| key.to_string())
        .collect::<HashSet<String>>();
    let typed_pkgs = match parsed {
        crate::Parsed::V1(manifest) => manifest
            .install
            .inner()
            .keys()
            .cloned()
            .collect::<HashSet<String>>(),
        crate::Parsed::V1_10_0(manifest) => manifest
            .install
            .inner()
            .keys()
            .cloned()
            .collect::<HashSet<String>>(),
    };
    let to_remove = raw_pkgs
        .difference(&typed_pkgs)
        .cloned()
        .collect::<HashSet<_>>();
    let to_add = typed_pkgs
        .difference(&raw_pkgs)
        .cloned()
        .collect::<HashSet<_>>();
    let added_or_removed = to_add.union(&to_remove).cloned().collect::<HashSet<_>>();
    let to_update = typed_pkgs
        .difference(&added_or_removed)
        .cloned()
        .collect::<HashSet<_>>();
    let should_be_original_pkgs = to_remove.union(&to_update).cloned().collect::<HashSet<_>>();
    debug_assert_eq!(should_be_original_pkgs, raw_pkgs);

    for pkg in to_remove {
        debug!(%pkg, "removing package");
        install_table.remove(&pkg);
    }
    for pkg in to_add.iter() {
        debug!(%pkg, "adding package");
        let mut inner_descriptor = InlineTable::new();
        inner_descriptor.set_dotted(true);
        let mut descriptor = Item::Value(Value::InlineTable(inner_descriptor));
        update_descriptor(
            descriptor
                .as_table_like_mut()
                .expect("toml inline table should be table-like"),
            pkg.as_str(),
            parsed,
        )?;
        install_table.insert(pkg, descriptor);
    }
    for pkg in to_update {
        debug!(%pkg, "updating package");
        let raw_descriptor = install_table
            .get_mut(&pkg)
            .ok_or(ManifestError::PackageNotFound(pkg.clone()))?
            .as_table_like_mut()
            .ok_or(ManifestError::Other(format!(
                "package descriptor '{}' was not a TOML table",
                pkg
            )))?;
        update_descriptor(raw_descriptor, pkg.as_str(), parsed)?;
    }

    Ok(())
}

/// Brings a raw TOML package descriptor into sync with the corresponding typed package descriptor.
fn update_descriptor(
    raw: &mut dyn toml_edit::TableLike,
    install_id: &str,
    parsed: &Parsed,
) -> Result<(), ManifestError> {
    match parsed {
        Parsed::V1(manifest) => {
            let typed = manifest
                .install
                .inner()
                .get(install_id)
                .ok_or(ManifestError::PackageNotFound(install_id.to_string()))?;
            use crate::parsed::v1::ManifestPackageDescriptor::*;
            match typed {
                Catalog(d) => update_v1_catalog_descriptor(raw, d),
                FlakeRef(d) => update_v1_flake_descriptor(raw, d),
                StorePath(d) => update_store_path_descriptor(raw, d),
            }
        },
        Parsed::V1_10_0(manifest) => {
            let typed = manifest
                .install
                .inner()
                .get(install_id)
                .ok_or(TomlEditError::PackageNotFound(install_id.to_string()))?;
            use crate::parsed::v1_10_0::ManifestPackageDescriptor::*;
            match typed {
                Catalog(d) => update_v1_10_0_catalog_descriptor(raw, d),
                FlakeRef(d) => update_v1_10_0_flake_descriptor(raw, d),
                StorePath(d) => update_store_path_descriptor(raw, d),
            }
        },
    }
    Ok(())
}

fn toml_string(s: impl AsRef<str>) -> Value {
    Value::String(Formatted::new(s.as_ref().to_string()))
}

fn toml_array_of_strings(strs: &[String]) -> Value {
    Value::Array(strs.iter().map(toml_string).collect::<Array>())
}

fn toml_priority(p: u64) -> Value {
    Value::Integer(Formatted::new(p as i64))
}

fn get_install_table_mut(doc: &mut DocumentMut) -> Result<&mut Table, TomlEditError> {
    let install_field = doc
        .entry("install")
        .or_insert_with(|| Item::Table(Table::new()));
    let install_field_type = install_field.type_name().into();
    install_field.as_table_mut().ok_or_else(|| {
        debug!("creating new [install] table");
        TomlEditError::MalformedInstallTable(install_field_type)
    })
}

fn update_v1_catalog_descriptor(
    raw: &mut dyn TableLike,
    descriptor: &v1::PackageDescriptorCatalog,
) {
    let v1::PackageDescriptorCatalog {
        pkg_path,
        pkg_group,
        priority,
        version,
        systems,
    } = descriptor;
    raw.insert("pkg-path", toml_string(pkg_path).into());
    if let Some(pkg_group) = pkg_group {
        raw.insert("pkg-group", toml_string(pkg_group).into());
    } else {
        raw.remove("pkg-group");
    }
    if let Some(priority) = priority {
        raw.insert("priority", toml_priority(*priority).into());
    } else {
        raw.remove("priority");
    }
    if let Some(version) = version {
        raw.insert("version", toml_string(version).into());
    } else {
        raw.remove("version");
    }
    if let Some(systems) = systems {
        raw.insert("systems", toml_array_of_strings(systems).into());
    } else {
        raw.remove("systems");
    }
}

fn update_v1_flake_descriptor(raw: &mut dyn TableLike, descriptor: &v1::PackageDescriptorFlake) {
    let v1::PackageDescriptorFlake {
        flake,
        priority,
        systems,
    } = descriptor;
    raw.insert("flake", toml_string(flake).into());
    if let Some(priority) = priority {
        raw.insert("priority", toml_priority(*priority).into());
    } else {
        raw.remove("priority");
    }
    if let Some(systems) = systems {
        raw.insert("systems", toml_array_of_strings(systems).into());
    } else {
        raw.remove("systems");
    }
}

fn update_store_path_descriptor(
    raw: &mut dyn TableLike,
    descriptor: &common::PackageDescriptorStorePath,
) {
    let common::PackageDescriptorStorePath {
        store_path,
        systems,
        priority,
    } = descriptor;
    raw.insert("store-path", toml_string(store_path).into());
    if let Some(priority) = priority {
        raw.insert("priority", toml_priority(*priority).into());
    } else {
        raw.remove("priority");
    }
    if let Some(systems) = systems {
        raw.insert("systems", toml_array_of_strings(systems).into());
    } else {
        raw.remove("systems");
    }
}

fn update_v1_10_0_catalog_descriptor(
    raw: &mut dyn TableLike,
    descriptor: &v1_10_0::PackageDescriptorCatalog,
) {
    let v1_10_0::PackageDescriptorCatalog {
        pkg_path,
        pkg_group,
        priority,
        version,
        systems,
        outputs,
    } = descriptor;
    raw.insert("pkg-path", toml_string(pkg_path).into());
    if let Some(pkg_group) = pkg_group {
        raw.insert("pkg-group", toml_string(pkg_group).into());
    } else {
        raw.remove("pkg-group");
    }
    if let Some(priority) = priority {
        raw.insert("priority", toml_priority(*priority).into());
    } else {
        raw.remove("priority");
    }
    if let Some(version) = version {
        raw.insert("version", toml_string(version).into());
    } else {
        raw.remove("version");
    }
    if let Some(systems) = systems {
        raw.insert("systems", toml_array_of_strings(systems).into());
    } else {
        raw.remove("systems");
    }
    if let Some(outputs) = outputs {
        match outputs {
            v1_10_0::SelectedOutputs::All(_) => {
                raw.insert("outputs", toml_string("all").into());
            },
            v1_10_0::SelectedOutputs::Specific(items) => {
                raw.insert("outputs", toml_array_of_strings(items).into());
            },
        }
    } else {
        raw.remove("outputs");
    }
}

fn update_v1_10_0_flake_descriptor(
    raw: &mut dyn TableLike,
    descriptor: &v1_10_0::PackageDescriptorFlake,
) {
    let v1_10_0::PackageDescriptorFlake {
        flake,
        priority,
        systems,
        outputs,
    } = descriptor;
    raw.insert("flake", toml_string(flake).into());
    if let Some(priority) = priority {
        raw.insert("priority", toml_priority(*priority).into());
    } else {
        raw.remove("priority");
    }
    if let Some(systems) = systems {
        raw.insert("systems", toml_array_of_strings(systems).into());
    } else {
        raw.remove("systems");
    }
    if let Some(outputs) = outputs {
        match outputs {
            v1_10_0::SelectedOutputs::All(_) => {
                raw.insert("outputs", toml_string("all").into());
            },
            v1_10_0::SelectedOutputs::Specific(items) => {
                raw.insert("outputs", toml_array_of_strings(items).into());
            },
        }
    } else {
        raw.remove("outputs");
    }
}

/// Add a `system` to the `[options.systems]` array of a manifest
pub fn add_system(toml: &str, system: &str) -> Result<DocumentMut, TomlEditError> {
    let mut doc = toml
        .parse::<DocumentMut>()
        .map_err(TomlEditError::ParseToml)?;

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

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use toml_edit::DocumentMut;

    use crate::lockfile::Lockfile;
    use crate::parsed::latest::ManifestLatest;
    use crate::parsed::v1::ManifestV1;
    use crate::{Manifest, Migrated, Parsed};

    pub fn mk_test_manifest_from_contents(s: impl AsRef<str>) -> Manifest<Migrated> {
        let toml = s.as_ref().parse::<DocumentMut>().unwrap();
        let typed: ManifestLatest = toml_edit::de::from_document(toml.clone()).unwrap();
        Manifest {
            inner: Migrated {
                // UNUSED, or DON'T USE, PICK ONE
                original_parsed: Parsed::V1(ManifestV1::default()),
                lockfile: Some(Lockfile::default()),
                // OK YOU CAN USE THESE
                migrated_raw: toml,
                migrated_parsed: typed,
            },
        }
    }

    pub fn empty_test_migrated_manifest() -> Manifest<Migrated> {
        let toml = "schema-version = \"1.10.0\""
            .parse::<DocumentMut>()
            .unwrap();
        let typed: ManifestLatest = toml_edit::de::from_document(toml.clone()).unwrap();
        Manifest {
            inner: Migrated {
                // UNUSED, or DON'T USE, PICK ONE
                original_parsed: Parsed::V1(ManifestV1::default()),
                lockfile: Some(Lockfile::default()),
                // OK YOU CAN USE THESE
                migrated_raw: toml,
                migrated_parsed: typed,
            },
        }
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use proptest_derive::Arbitrary;

    use super::*;
    use crate::interfaces::CommonFields;
    use crate::raw::test_helpers::mk_test_manifest_from_contents;
    use crate::test_helpers::with_latest_schema;

    const DUMMY_MANIFEST: &str = indoc! {r#"
        schema-version = "1.10.0"

        [install]
        hello.pkg-path = "hello"

        [install.ripgrep]
        pkg-path = "ripgrep"
        [install.bat]
        pkg-path = "bat"
    "#};

    fn dummy_manifest() -> Manifest<Migrated> {
        mk_test_manifest_from_contents(DUMMY_MANIFEST)
    }

    /// Check whether a TOML document contains a line declaring that the provided package
    /// should be installed.
    pub fn contains_package(toml: &DocumentMut, pkg_name: &str) -> bool {
        toml.get("install")
            .and_then(|item| item.as_table())
            .map(|table| table.contains_key(pkg_name))
            .unwrap()
    }

    #[test]
    fn insert_adds_new_package() {
        let test_packages = vec![PackageToInstall::Catalog(
            CatalogPackage::from_str("python").unwrap(),
        )];
        let pre_addition_manifest = dummy_manifest();
        let pre_addition_toml = pre_addition_manifest.inner.migrated_raw.clone();
        assert!(!contains_package(&pre_addition_toml, test_packages[0].id()));
        let insertion = pre_addition_manifest.add_packages(&test_packages).unwrap();
        assert!(
            insertion.new_manifest.is_some(),
            "manifest was changed by install"
        );
        let new_toml = insertion.new_manifest.unwrap().inner.migrated_raw;
        assert!(contains_package(&new_toml, test_packages[0].id()));
    }

    #[test]
    fn no_change_adding_existing_package() {
        let test_packages = vec![PackageToInstall::Catalog(
            CatalogPackage::from_str("hello").unwrap(),
        )];
        let pre_addition_manifest = dummy_manifest();
        let pre_addition_toml = pre_addition_manifest.inner.migrated_raw.clone();
        // dummy manifest already contains `hello`
        assert!(contains_package(&pre_addition_toml, test_packages[0].id()));
        let insertion = pre_addition_manifest.add_packages(&test_packages).unwrap();
        assert!(
            insertion.new_manifest.is_none(),
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
        let manifest = dummy_manifest();
        let insertion = manifest.add_packages(&test_packages).unwrap();
        let toml = insertion.new_manifest.unwrap().inner.migrated_raw;
        assert!(contains_package(&toml, test_packages[0].id()));
        assert!(
            !insertion.already_installed.values().all(|p| *p),
            "none of the packages should be listed as already installed"
        );
    }

    #[test]
    fn removes_all_requested_packages() {
        let test_packages = vec!["hello".to_owned(), "ripgrep".to_owned()];
        let manifest = dummy_manifest();
        let post_removal = manifest.remove_packages(&test_packages).unwrap();
        let toml = post_removal.inner.migrated_raw;
        assert!(!contains_package(&toml, "hello"));
        assert!(!contains_package(&toml, "ripgrep"));
    }

    #[test]
    fn error_when_removing_nonexistent_package() {
        let test_packages = vec![
            "hello".to_owned(),
            "DOES_NOT_EXIST".to_owned(),
            "nodePackages.@".to_owned(),
        ];
        let manifest = dummy_manifest();
        let removal = manifest.remove_packages(&test_packages);
        assert!(matches!(removal, Err(ManifestError::PackageNotFound(_))));
    }

    #[test]
    fn inserts_package_needing_quotes() {
        let attrs = r#"foo."bar.baz".qux"#;
        let test_packages = vec![PackageToInstall::Catalog(
            CatalogPackage::from_str(attrs).unwrap(),
        )];
        let pre_addition = dummy_manifest();
        assert!(!contains_package(
            &pre_addition.inner.migrated_raw,
            test_packages[0].id()
        ));
        let insertion = pre_addition
            .add_packages(&test_packages)
            .expect("couldn't add package");
        assert!(
            insertion.new_manifest.is_some(),
            "manifest was changed by install"
        );
        let new_toml = insertion.new_manifest.unwrap().inner.migrated_raw;
        assert!(contains_package(&new_toml, test_packages[0].id()));
        let inserted_path = new_toml["install"]["qux"]["pkg-path"].as_str().unwrap();
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
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), false);

        let parsed: CatalogPackage = "foo.bar@=1.2.3".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "bar".to_string(),
            pkg_path: "foo.bar".to_string(),
            version: Some("=1.2.3".to_string()),
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), false);

        let parsed: CatalogPackage = "foo.bar@23.11".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "bar".to_string(),
            pkg_path: "foo.bar".to_string(),
            version: Some("23.11".to_string()),
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), false);

        let parsed: CatalogPackage = "rubyPackages.\"http_parser.rb\"".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "\"http_parser.rb\"".to_string(),
            pkg_path: "rubyPackages.\"http_parser.rb\"".to_string(),
            version: None,
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), false);

        // First part contains a dot (should be treated as attr-path)
        let parsed: CatalogPackage = "nodePackages.@angular/cli".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "cli".to_string(),
            pkg_path: "nodePackages.@angular/cli".to_string(),
            version: None,
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), false);

        // Complex package name with dots and special characters, ugly but valid
        let parsed: CatalogPackage =
            "nodePackages.tedicross-git+https://github.com/TediCross/TediCross.git#v0.8.7"
                .parse()
                .unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "7".to_string(),
            pkg_path:
                "nodePackages.tedicross-git+https://github.com/TediCross/TediCross.git#v0.8.7"
                    .to_string(),
            version: None,
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), false);

        // Attributes starting with `@` are allowed, the @ is not delimting the version if following a '.'
        let parsed: CatalogPackage = "nodePackages.@angular@1.2.3".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "@angular".to_string(),
            pkg_path: "nodePackages.@angular".to_string(),
            version: Some("1.2.3".to_string()),
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), false);

        // Attributes starting with `@` are allowed, the @ is not delimting the version
        // if its the first character
        let parsed: CatalogPackage = "@1.2.3".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "3".to_string(),
            pkg_path: "@1.2.3".to_string(),
            version: None,
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), false);

        // Attributes starting with `@` are allowed, the @ is not delimting the version
        // if its the first character.
        // Following `@` may delimit a version
        let parsed: CatalogPackage = "@pkg@version".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "@pkg".to_string(),
            pkg_path: "@pkg".to_string(),
            version: Some("version".to_string()),
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), false);

        // Package from custom catalog
        let parsed: CatalogPackage = "mycatalog/foo".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "foo".to_string(),
            pkg_path: "mycatalog/foo".to_string(),
            version: None,
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), true);

        // Package with dotted path and custom catalog
        let parsed: CatalogPackage = "mycatalog/foo.bar".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "bar".to_string(),
            pkg_path: "mycatalog/foo.bar".to_string(),
            version: None,
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), true);

        // Package with nested path and custom catalog
        let parsed: CatalogPackage = "mycatalog/category/package".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "package".to_string(),
            pkg_path: "mycatalog/category/package".to_string(),
            version: None,
            systems: None,
            outputs: None,
        });
        assert_eq!(parsed.is_custom_catalog(), true);

        CatalogPackage::from_str("foo.\"bar.baz.qux@1.2.3")
            .expect_err("missing closing quote should cause failure");
        CatalogPackage::from_str("foo@").expect_err("missing version should cause failure");
    }

    #[test]
    fn manifest_is_updated_correctly_with_outputs() {
        let package = PackageToInstall::parse(&"".to_string(), "curl^bin,man").unwrap();
        let contents = "
schema-version = \"1.10.0\"
        ";
        let manifest = mk_test_manifest_from_contents(contents);
        let insertion = manifest
            .add_packages(&[package])
            .expect("couldn't add package");
        assert_eq!(
            insertion
                .new_manifest
                .unwrap()
                .inner
                .migrated_raw
                .to_string(),
            "
schema-version = \"1.10.0\"

[install]
curl.pkg-path = \"curl\"
curl.outputs = [\"bin\", \"man\"]
        "
        );
    }

    #[test]
    fn parses_descriptors_with_outputs() {
        // Package with specific outputs
        let parsed: CatalogPackage = "curl^bin,man".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "curl".to_string(),
            pkg_path: "curl".to_string(),
            version: None,
            systems: None,
            outputs: Some(RawSelectedOutputs::Specific(vec![
                "bin".to_string(),
                "man".to_string()
            ])),
        });

        // Package with all outputs
        let parsed: CatalogPackage = "curl^..".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "curl".to_string(),
            pkg_path: "curl".to_string(),
            version: None,
            systems: None,
            outputs: Some(RawSelectedOutputs::All),
        });

        // Package with version containing special characters
        let parsed: CatalogPackage = "nodePackages.typescript@^5.0.0".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "typescript".to_string(),
            pkg_path: "nodePackages.typescript".to_string(),
            version: Some("^5.0.0".to_string()),
            systems: None,
            outputs: None,
        });

        // Invalid package with version and outputs
        let parsed: CatalogPackage = "nodePackages.typescript@5.0^bin,man,dev".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "typescript".to_string(),
            pkg_path: "nodePackages.typescript".to_string(),
            version: Some("5.0^bin,man,dev".to_string()),
            systems: None,
            outputs: None,
        });

        // Package with outputs containing spaces (should be trimmed)
        let parsed: CatalogPackage = "curl^bin, man , dev".parse().unwrap();
        assert_eq!(parsed, CatalogPackage {
            id: "curl".to_string(),
            pkg_path: "curl".to_string(),
            version: None,
            systems: None,
            outputs: Some(RawSelectedOutputs::Specific(vec![
                "bin".to_string(),
                "man".to_string(),
                "dev".to_string()
            ])),
        });

        // Error: empty outputs specification
        CatalogPackage::from_str("curl^").expect_err("empty outputs should cause failure");
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
    fn update_systems_does_not_add_options_table_when_systems_is_none() {
        let toml_str = with_latest_schema(indoc! {r#"
            [install]
            hello.pkg-path = "hello"
        "#});
        let mut manifest = Manifest::parse_toml_typed(&toml_str).unwrap();
        manifest.update_systems().unwrap();
        assert!(
            manifest.inner.raw.get("options").is_none(),
            "update_systems with None should not create an [options] table"
        );
    }

    #[test]
    fn update_systems_adds_options_table_when_systems_provided() {
        let toml_str = with_latest_schema(indoc! {r#"
            [install]
            hello.pkg-path = "hello"
        "#});
        let mut manifest = Manifest::parse_toml_typed(&toml_str).unwrap();
        let systems = vec!["x86_64-linux".to_string()];
        manifest.options_mut().systems = Some(systems.clone());
        manifest.update_systems().unwrap();
        let updated_systems = manifest.inner.raw["options"]["systems"]
            .as_array()
            .unwrap()
            .into_iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(updated_systems, systems);
    }

    #[test]
    fn update_systems_removes_systems_without_removing_other_options() {
        let toml_str = with_latest_schema(indoc! {r#"
            [options]
            systems = ["x86_64-linux"]
            allow.unfree = true
        "#});
        let mut manifest = Manifest::parse_toml_typed(&toml_str).unwrap();
        manifest.options_mut().systems = None;
        manifest.update_systems().unwrap();
        let opts = manifest.inner.raw["options"].clone();
        assert!(opts["allow"]["unfree"].as_bool().unwrap());
        assert!(opts.get("systems").is_none());
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
}
