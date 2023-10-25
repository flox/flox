use std::io;
use std::path::{Path, PathBuf};

use toml_edit::{Document, Item, Table};

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("couldn't open the manifest at path {0}: {1}")]
    OpenManifest(PathBuf, io::Error),
    #[error("couldn't parse manifest contents: {0}")]
    ParseManifest(toml_edit::TomlError),
    #[error("package already installed")]
    AlreadyInstalled,
    #[error("'install' must be a table, but found {0} instead")]
    MalformedManifest(String),
    #[error("couldn't write modified manifest: {0}")]
    WriteManifest(io::Error),
}

pub fn install(
    manifest_path: &impl AsRef<Path>,
    pkg_name: &impl AsRef<str>,
) -> Result<(), InstallError> {
    let manifest_path = manifest_path.as_ref();
    let pkg_name = pkg_name.as_ref();
    let contents = std::fs::read_to_string(manifest_path)
        .map_err(|e| InstallError::OpenManifest(manifest_path.to_path_buf(), e))?;
    let toml = insert_package(&contents, pkg_name)?;
    std::fs::write(manifest_path, toml.to_string()).map_err(InstallError::WriteManifest)?;
    Ok(())
}

fn insert_package(manifest_contents: &str, pkg_name: &str) -> Result<Document, InstallError> {
    let mut toml = manifest_contents
        .parse::<Document>()
        .map_err(InstallError::ParseManifest)?;
    match toml.entry("install") {
        toml_edit::Entry::Occupied(ref mut existing_installs) => {
            if let Item::Table(ref mut installs) = existing_installs.get_mut() {
                if installs.contains_key(pkg_name) {
                    Err(InstallError::AlreadyInstalled)
                } else {
                    installs.insert(pkg_name, Item::Table(Table::new()));
                    Ok(toml)
                }
            } else {
                return Err(InstallError::MalformedManifest(
                    existing_installs.get().type_name().into(),
                ));
            }
        },
        toml_edit::Entry::Vacant(empty_installs) => {
            let mut installs_table = Table::new();
            installs_table.insert(pkg_name, Item::Table(Table::new()));
            empty_installs.insert(Item::Table(installs_table));
            Ok(toml)
        },
    }
}

// TODO: will be used in uninstall
#[allow(unused)]
fn contains_package(toml: &Document, pkg_name: &str) -> Result<bool, InstallError> {
    if let Some(installs) = toml.get("install") {
        if let Item::Table(installs_table) = installs {
            Ok(installs_table.contains_key(pkg_name))
        } else {
            Err(InstallError::MalformedManifest(installs.type_name().into()))
        }
    } else {
        Ok(false)
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
        let test_package = "python";
        let pre_addition_toml = DUMMY_MANIFEST.parse::<Document>().unwrap();
        assert!(!contains_package(&pre_addition_toml, test_package).unwrap());
        let toml = insert_package(DUMMY_MANIFEST, test_package).expect("couldn't add package");
        assert!(contains_package(&toml, test_package).unwrap());
        eprintln!("{}", toml);
    }

    #[test]
    fn install_error_adding_existing_package() {
        let test_package = "hello";
        let pre_addition_toml = DUMMY_MANIFEST.parse::<Document>().unwrap();
        assert!(contains_package(&pre_addition_toml, test_package).unwrap());
        let attempted_install = insert_package(DUMMY_MANIFEST, test_package);
        assert!(matches!(
            attempted_install,
            Err(InstallError::AlreadyInstalled)
        ))
    }

    #[test]
    fn install_adds_install_table_when_missing() {
        let test_package = "foo";
        let toml = insert_package("", test_package).unwrap();
        assert!(contains_package(&toml, test_package).unwrap());
    }

    #[test]
    fn install_error_when_manifest_malformed() {
        let test_package = "foo";
        let attempted_install = insert_package(BAD_MANIFEST, test_package);
        assert!(matches!(
            attempted_install,
            Err(InstallError::MalformedManifest(_))
        ))
    }
}
