use std::collections::HashMap;

use log::debug;
use toml_edit::{self, Document, InlineTable, Item, Table, Value};

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
}

/// Records the result of trying to install a collection of packages to the
#[derive(Debug)]
pub struct PackageInsertion {
    pub new_toml: Option<Document>,
    pub already_installed: HashMap<String, bool>,
}

/// Insert package names into the `[install]` table of a manifest.
///
/// Packages are always inserted as "tables", meaning a package `foo` will appear
/// as `[install.foo]`. The form you may be more familiar with:
/// ```toml
/// [install]
/// foo = {}
/// ```
/// is called an "inline table" and the `toml_edit` library has middling support
/// for entries in this form.
pub fn insert_packages(
    manifest_contents: &str,
    pkgs: impl Iterator<Item = String>,
) -> Result<PackageInsertion, TomlEditError> {
    debug!("attempting to insert packages into manifest");
    let mut already_installed: HashMap<String, bool> = HashMap::new();
    let mut toml = manifest_contents
        .parse::<Document>()
        .map_err(TomlEditError::ParseManifest)?;
    match toml.entry("install") {
        toml_edit::Entry::Occupied(ref mut existing_installs) => {
            debug!("editing existing [install] table");
            if let Item::Table(ref mut installs) = existing_installs.get_mut() {
                for pkg in pkgs {
                    if !installs.contains_key(&pkg) {
                        installs.insert(&pkg, Item::Value(Value::InlineTable(InlineTable::new())));
                        already_installed.insert(pkg.clone(), false);
                        debug!("package {pkg} newly installed");
                    } else {
                        already_installed.insert(pkg.clone(), true);
                        debug!("package {pkg} already installed");
                    }
                }

                // TODO: Figure out a better sorting system
                // installs.sort_values_by(|key1, _, key2, _| key1.cmp(key2));
            } else {
                return Err(TomlEditError::MalformedInstallTable(
                    existing_installs.get().type_name().into(),
                ));
            }
        },
        toml_edit::Entry::Vacant(empty_installs) => {
            debug!("creating new [install] table");
            let mut installs_table = Table::new();
            for pkg in pkgs {
                installs_table.insert(&pkg, Item::Value(Value::InlineTable(InlineTable::new())));
                already_installed.insert(pkg.clone(), false);
            }
            // TODO: Figure out a better sorting system
            // installs_table.sort_values_by(|key1, _, key2, _| key1.cmp(key2));
            empty_installs.insert(Item::Table(installs_table));
        },
    };
    Ok(PackageInsertion {
        new_toml: if !already_installed.values().all(|p| *p) {
            Some(toml)
        } else {
            None
        },
        already_installed,
    })
}

// FIXME: will be used in uninstall
/// Check whether a TOML document contains a line declaring that the provided package
/// should be installed.
#[allow(unused)]
pub fn contains_package(toml: &Document, pkg_name: &str) -> Result<bool, TomlEditError> {
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

/// List the packages contained in the contents of a manifest.
pub fn list_packages(manifest_contents: &str) -> Result<Option<Vec<String>>, TomlEditError> {
    let toml = manifest_contents
        .parse::<Document>()
        .map_err(TomlEditError::ParseManifest)?;
    if let Some(Item::Table(installs)) = toml.get("install") {
        let pkgs = installs
            .iter()
            .map(|(pkg, _)| pkg.to_string())
            .collect::<Vec<_>>();
        if pkgs.is_empty() {
            Ok(None)
        } else {
            Ok(Some(pkgs))
        }
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod test {
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
    fn install_adds_new_package() {
        let test_packages = vec!["python".to_owned()];
        let pre_addition_toml = DUMMY_MANIFEST.parse::<Document>().unwrap();
        assert!(!contains_package(&pre_addition_toml, &test_packages[0]).unwrap());
        let insertion = insert_packages(DUMMY_MANIFEST, test_packages.iter().cloned())
            .expect("couldn't add package");
        assert!(
            insertion.new_toml.is_some(),
            "manifest was changed by install"
        );
        assert!(contains_package(&insertion.new_toml.unwrap(), &test_packages[0]).unwrap());
    }

    #[test]
    fn no_change_adding_existing_package() {
        let test_packages = vec!["hello".to_owned()];
        let pre_addition_toml = DUMMY_MANIFEST.parse::<Document>().unwrap();
        assert!(contains_package(&pre_addition_toml, &test_packages[0]).unwrap());
        let insertion = insert_packages(DUMMY_MANIFEST, test_packages.iter().cloned()).unwrap();
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
    fn install_adds_install_table_when_missing() {
        let test_packages = vec!["foo".to_owned()];
        let insertion = insert_packages("", test_packages.iter().cloned()).unwrap();
        assert!(contains_package(&insertion.new_toml.clone().unwrap(), &test_packages[0]).unwrap());
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
    fn install_error_when_manifest_malformed() {
        let test_packages = vec!["foo".to_owned()];
        let attempted_install = insert_packages(BAD_MANIFEST, test_packages.iter().cloned());
        assert!(matches!(
            attempted_install,
            Err(TomlEditError::MalformedInstallTable(_))
        ))
    }
}
