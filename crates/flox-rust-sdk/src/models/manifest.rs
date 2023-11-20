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
    /// The `[install]` table was missing entirely
    #[error("'install' table not found")]
    MissingInstallTable,
    /// Tried to uninstall a package that wasn't installed
    #[error("couldn't uninstall '{0}', wasn't previously installed")]
    PackageNotFound(String),
}

/// Records the result of trying to install a collection of packages to the
#[derive(Debug)]
pub struct PackageInsertion {
    pub new_toml: Option<Document>,
    pub already_installed: HashMap<String, bool>,
}

/// Insert package names into the `[install]` table of a manifest.
pub fn insert_packages(
    manifest_contents: &str,
    pkgs: impl Iterator<Item = String>,
) -> Result<PackageInsertion, TomlEditError> {
    debug!("attempting to insert packages into manifest");
    let mut already_installed: HashMap<String, bool> = HashMap::new();
    let mut toml = manifest_contents
        .parse::<Document>()
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
        if !install_table.contains_key(&pkg) {
            install_table.insert(&pkg, Item::Value(Value::InlineTable(InlineTable::new())));
            already_installed.insert(pkg.clone(), false);
            debug!("package '{pkg}' newly installed");
        } else {
            already_installed.insert(pkg.clone(), true);
            debug!("package '{pkg}' already installed");
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
    pkgs: impl Iterator<Item = String>,
) -> Result<Document, TomlEditError> {
    debug!("attempting to remove packages from the manifest");
    let mut toml = manifest_contents
        .parse::<Document>()
        .map_err(TomlEditError::ParseManifest)?;

    let installs_table = {
        let installs_field = toml
            .get_mut("install")
            .ok_or(TomlEditError::MissingInstallTable)?;

        let type_name = installs_field.type_name().into();

        installs_field
            .as_table_mut()
            .ok_or(TomlEditError::MalformedInstallTable(type_name))?
    };

    for pkg in pkgs {
        debug!("checking for presence of package '{pkg}'");
        if !installs_table.contains_key(&pkg) {
            debug!("package '{pkg}' wasn't found");
            return Err(TomlEditError::PackageNotFound(pkg.clone()));
        } else {
            installs_table.remove(&pkg);
            debug!("package '{pkg}' was removed");
        }
    }

    Ok(toml)
}

/// Check whether a TOML document contains a line declaring that the provided package
/// should be installed.
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
    fn insert_adds_new_package() {
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
    fn insert_adds_install_table_when_missing() {
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
    fn insert_error_when_manifest_malformed() {
        let test_packages = vec!["foo".to_owned()];
        let attempted_insertion = insert_packages(BAD_MANIFEST, test_packages.iter().cloned());
        assert!(matches!(
            attempted_insertion,
            Err(TomlEditError::MalformedInstallTable(_))
        ))
    }

    #[test]
    fn remove_error_when_manifest_malformed() {
        let test_packages = vec!["hello".to_owned()];
        let attempted_removal = remove_packages(BAD_MANIFEST, test_packages.iter().cloned());
        assert!(matches!(
            attempted_removal,
            Err(TomlEditError::MalformedInstallTable(_))
        ))
    }

    #[test]
    fn error_when_install_table_missing() {
        let test_packages = vec!["hello".to_owned()];
        let removal = remove_packages("", test_packages.iter().cloned());
        assert!(matches!(removal, Err(TomlEditError::MissingInstallTable)));
    }

    #[test]
    fn removes_all_requested_packages() {
        let test_packages = vec!["hello".to_owned(), "ripgrep".to_owned()];
        let toml = remove_packages(DUMMY_MANIFEST, test_packages.iter().cloned()).unwrap();
        assert!(!contains_package(&toml, "hello").unwrap());
        assert!(!contains_package(&toml, "ripgrep").unwrap());
    }

    #[test]
    fn error_when_removing_nonexistent_package() {
        let test_packages = vec!["hello".to_owned(), "DOES_NOT_EXIST".to_owned()];
        let removal = remove_packages(DUMMY_MANIFEST, test_packages.iter().cloned());
        assert!(matches!(removal, Err(TomlEditError::PackageNotFound(_))));
    }
}
