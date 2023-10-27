use std::path::{Path, PathBuf};
use std::{fs, io};

use async_trait::async_trait;
use flox_types::catalog::{CatalogEntry, EnvCatalog, System};
use flox_types::version::Version;
use runix::command_line::{NixCommandLine, NixCommandLineRunError, NixCommandLineRunJsonError};
use runix::installable::FlakeAttribute;
use runix::store_path::StorePath;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use toml_edit::{Document, Item, Table};
use walkdir::WalkDir;

use self::managed_environment::ManagedEnvironmentError;
use super::environment_ref::{
    EnvironmentName,
    EnvironmentOwner,
    EnvironmentRef,
    EnvironmentRefError,
};
use super::flox_package::{FloxPackage, FloxTriple};
use crate::utils::copy_file_without_permissions;
use crate::utils::errors::IoError;

pub mod managed_environment;
pub mod path_environment;
pub mod remote_environment;

pub const CATALOG_JSON: &str = "catalog.json";
pub const DOT_FLOX: &str = ".flox";
pub const ENVIRONMENT_POINTER_FILENAME: &str = "env.json";
pub const MANIFEST_FILENAME: &str = "manifest.toml";
// don't forget to update the man page
pub const DEFAULT_KEEP_GENERATIONS: usize = 10;
// don't forget to update the man page
pub const DEFAULT_MAX_AGE_DAYS: u32 = 90;

pub enum InstalledPackage {
    Catalog(FloxTriple, CatalogEntry),
    FlakeAttribute(FlakeAttribute, CatalogEntry),
    StorePath(StorePath),
}

#[async_trait]
pub trait Environment {
    /// Build the environment and create a result link as gc-root
    async fn build(
        &mut self,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<(), EnvironmentError2>;

    /// Install packages to the environment atomically
    async fn install(
        &mut self,
        packages: Vec<String>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<bool, EnvironmentError2>;

    /// Uninstall packages from the environment atomically
    async fn uninstall(
        &mut self,
        packages: Vec<FloxPackage>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<bool, EnvironmentError2>;

    /// Atomically edit this environment, ensuring that it still builds
    async fn edit(
        &mut self,
        nix: &NixCommandLine,
        system: System,
        contents: String,
    ) -> Result<(), EnvironmentError2>;

    async fn catalog(
        &self,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<EnvCatalog, EnvironmentError2>;

    /// Extract the current content of the manifest
    fn manifest_content(&self) -> Result<String, EnvironmentError2>;

    /// Return the [EnvironmentRef] for the environment for identification
    fn environment_ref(&self) -> EnvironmentRef;

    /// Return a flake attribute installable for this environment
    // TODO consider removing this from the trait
    fn flake_attribute(&self, system: System) -> FlakeAttribute;

    /// Returns the environment owner
    fn owner(&self) -> Option<EnvironmentOwner>;

    /// Returns the environment name
    fn name(&self) -> EnvironmentName;

    /// Delete the Environment
    fn delete(self) -> Result<(), EnvironmentError2>
    where
        Self: Sized;

    /// Remove gc-roots
    // TODO consider renaming or removing - we might not support this for PathEnvironment
    fn delete_symlinks(&self) -> Result<bool, EnvironmentError2> {
        Ok(false)
    }
}

/// A pointer to an environment, either managed or path.
/// This is used to determine the type of an environment at a given path.
/// See [EnvironmentPointer::open].
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum EnvironmentPointer {
    /// Identifies an environment whose source of truth lies outside of the project itself
    Managed(ManagedPointer),
    /// Identifies an environment whose source of truth lies inside the project
    Path(PathPointer),
}

/// The identifier for a project environment.
///
/// This is serialized to `env.json` inside the `.flox` directory
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PathPointer {
    pub name: EnvironmentName,
    version: Version<1>,
}

impl PathPointer {
    /// Create a new [PathPointer] with the given name.
    pub fn new(name: EnvironmentName) -> Self {
        Self {
            name,
            version: Version::<1>,
        }
    }
}

/// The identifier for an environment that's defined outside of the project itself, and
/// points to an environment owner and the name of the environment.
///
/// This is serialized to an `env.json` inside the `.flox` directory.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ManagedPointer {
    pub owner: EnvironmentOwner,
    pub name: EnvironmentName,
    version: Version<1>,
}

impl EnvironmentPointer {
    /// The function attempts to open an environment at the specified path
    /// by reading the contents of a file named .flox/[ENVIRONMENT_POINTER_FILENAME].
    /// If the file is found and its contents can be deserialized,
    /// the function returns an [EnvironmentPointer] containing information about the environment.
    /// If reading or parsing the file fails, an [EnvironmentError2] is returned.
    ///
    /// Use this method to determine the type of an environment at a given path.
    /// The result should be used to call the appropriate `open` method
    /// on either [PathEnvironment] or [ManagedEnvironment].
    pub fn open(path: impl AsRef<Path>) -> Result<EnvironmentPointer, EnvironmentError2> {
        let dot_flox_path = path.as_ref().join(DOT_FLOX);
        let pointer_path = dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME);
        let pointer_contents = match fs::read(pointer_path) {
            Ok(contents) => contents,
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => Err(EnvironmentError2::EnvNotFound)?,
                _ => Err(EnvironmentError2::ReadEnvironmentMetadata(err))?,
            },
        };

        serde_json::from_slice(&pointer_contents).map_err(EnvironmentError2::ParseEnvJson)
    }
}

#[derive(Debug, Error)]
pub enum EnvironmentError2 {
    #[error("ParseEnvRef")]
    ParseEnvRef(#[from] EnvironmentRefError),
    #[error("EmptyDotFlox")]
    EmptyDotFlox,
    #[error("DotFloxCanonicalize({0})")]
    EnvCanonicalize(std::io::Error),
    #[error("ReadDotFlox({0})")]
    ReadDotFlox(std::io::Error),
    #[error("ReadEnvDir({0})")]
    ReadEnvDir(std::io::Error),
    #[error("MakeSandbox({0})")]
    MakeSandbox(std::io::Error),
    #[error("DeleteEnvironment({0})")]
    DeleteEnvironment(std::io::Error),
    #[error("DotFloxNotFound")]
    DotFloxNotFound,
    #[error("InitEnv({0})")]
    InitEnv(std::io::Error),
    #[error("EnvNotFound")]
    EnvNotFound,
    #[error("EnvNotADirectory")]
    EnvNotADirectory,
    #[error("DirectoryNotAnEnv")]
    DirectoryNotAnEnv,
    #[error("EnvironmentExists")]
    EnvironmentExists,
    #[error("EvalCatalog({0})")]
    EvalCatalog(NixCommandLineRunJsonError),
    #[error("ParseCatalog({0})")]
    ParseCatalog(serde_json::Error),
    #[error("WriteCatalog({0})")]
    WriteCatalog(std::io::Error),
    #[error("Build({0})")]
    Build(NixCommandLineRunError),
    #[error("ReadManifest({0})")]
    ReadManifest(std::io::Error),
    #[error("ReadEnvironmentMetadata({0})")]
    ReadEnvironmentMetadata(std::io::Error),
    #[error("MakeTemporaryEnv({0})")]
    MakeTemporaryEnv(std::io::Error),
    #[error("UpdateManifest({0})")]
    UpdateManifest(std::io::Error),
    #[error("OpenManifest({0})")]
    OpenManifest(std::io::Error),
    #[error("Activate({0})")]
    Activate(NixCommandLineRunError),
    #[error("Prior transaction in progress. Delete {0} to discard.")]
    PriorTransaction(PathBuf),
    #[error("Failed to create backup for transaction: {0}")]
    BackupTransaction(std::io::Error),
    #[error("Failed to move modified environment into place: {0}")]
    Move(std::io::Error),
    #[error("Failed to abort transaction; backup could not be moved back into place: {0}")]
    AbortTransaction(std::io::Error),
    #[error("Failed to remove transaction backup: {0}")]
    RemoveBackup(std::io::Error),
    #[error("Failed to copy file")]
    CopyFile(IoError),
    #[error("Failed parsing contents of env.json file: {0}")]
    ParseEnvJson(serde_json::Error),
    #[error("Failed serializing contents of env.json file: {0}")]
    SerializeEnvJson(serde_json::Error),
    #[error("Failed write env.json file: {0}")]
    WriteEnvJson(std::io::Error),
    #[error(transparent)]
    ManagedEnvironment(#[from] ManagedEnvironmentError),
    #[error(transparent)]
    Install(#[from] InstallError),
}

/// Copy a whole directory recursively ignoring the original permissions
///
/// We need this because:
/// 1. Sometimes we need to copy from the Nix store
/// 2. fs_extra::dir::copy doesn't handle symlinks.
///    See: https://github.com/webdesus/fs_extra/issues/61
fn copy_dir_recursive(
    from: &impl AsRef<Path>,
    to: &impl AsRef<Path>,
    keep_permissions: bool,
) -> Result<(), std::io::Error> {
    for entry in WalkDir::new(from).into_iter().skip(1) {
        let entry = entry.unwrap();
        let new_path = to.as_ref().join(entry.path().strip_prefix(from).unwrap());
        match entry.file_type() {
            file_type if file_type.is_dir() => {
                std::fs::create_dir(new_path).unwrap();
            },
            file_type if file_type.is_symlink() => {
                let target = std::fs::read_link(entry.path())
                // we know the path exists and is a symlink
                .unwrap();
                // If target is a relative symlink, this will potentially orphan
                // it. But we're assuming it's absolute since we only copy links
                // to the Nix store.
                std::os::unix::fs::symlink(target, &new_path)?;
                // TODO handle permissions
            },
            _ => {
                if keep_permissions {
                    fs::copy(entry.path(), &new_path)?;
                } else {
                    copy_file_without_permissions(entry.path(), &new_path).unwrap();
                }
            },
        }
    }
    Ok(())
}

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

pub fn insert_packages(
    manifest_contents: &str,
    pkgs: impl Iterator<Item = String>,
) -> Result<(Document, bool), InstallError> {
    let mut changed = false;
    let mut toml = manifest_contents
        .parse::<Document>()
        .map_err(InstallError::ParseManifest)?;
    match toml.entry("install") {
        toml_edit::Entry::Occupied(ref mut existing_installs) => {
            if let Item::Table(ref mut installs) = existing_installs.get_mut() {
                for pkg in pkgs {
                    if !installs.contains_key(&pkg) {
                        installs.insert(&pkg, Item::Table(Table::new()));
                        changed = true;
                    }
                }
                // TODO: Figure out a better sorting system
                // installs.sort_values_by(|key1, _, key2, _| key1.cmp(key2));
                Ok((toml, changed))
            } else {
                return Err(InstallError::MalformedManifest(
                    existing_installs.get().type_name().into(),
                ));
            }
        },
        toml_edit::Entry::Vacant(empty_installs) => {
            changed = true;
            let mut installs_table = Table::new();
            for pkg in pkgs {
                installs_table.insert(&pkg, Item::Table(Table::new()));
            }
            // TODO: Figure out a better sorting system
            // installs_table.sort_values_by(|key1, _, key2, _| key1.cmp(key2));
            empty_installs.insert(Item::Table(installs_table));
            Ok((toml, changed))
        },
    }
}

// FIXME: will be used in uninstall
#[allow(unused)]
pub fn contains_package(toml: &Document, pkg_name: &str) -> Result<bool, InstallError> {
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
    use std::str::FromStr;

    use super::*;

    const MANAGED_ENV_JSON: &'_ str = r#"{
        "name": "name",
        "owner": "owner",
        "version": 1
    }"#;

    const PATH_ENV_JSON: &'_ str = r#"{
        "name": "name",
        "version": 1
    }"#;

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
    fn serializes_managed_environment_pointer() {
        let managed_pointer = EnvironmentPointer::Managed(ManagedPointer {
            name: EnvironmentName::from_str("name").unwrap(),
            owner: EnvironmentOwner::from_str("owner").unwrap(),
            version: Version::<1> {},
        });

        let json = serde_json::to_string(&managed_pointer).unwrap();
        // Convert both to `serde_json::Value` to test equality without worrying about whitespace
        let roundtrip_value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let example_value: serde_json::Value = serde_json::from_str(MANAGED_ENV_JSON).unwrap();
        assert_eq!(roundtrip_value, example_value);
    }

    #[test]
    fn deserializes_managed_environment_pointer() {
        let managed_pointer: EnvironmentPointer = serde_json::from_str(MANAGED_ENV_JSON).unwrap();
        assert_eq!(
            managed_pointer,
            EnvironmentPointer::Managed(ManagedPointer {
                name: EnvironmentName::from_str("name").unwrap(),
                owner: EnvironmentOwner::from_str("owner").unwrap(),
                version: Version::<1> {},
            })
        );
    }

    #[test]
    fn serializes_path_environment_pointer() {
        let path_pointer = EnvironmentPointer::Path(PathPointer {
            name: EnvironmentName::from_str("name").unwrap(),
            version: Version::<1> {},
        });

        let json = serde_json::to_string(&path_pointer).unwrap();
        // Convert both to `serde_json::Value` to test equality without worrying about whitespace
        let roundtrip_value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let example_value: serde_json::Value = serde_json::from_str(PATH_ENV_JSON).unwrap();
        assert_eq!(roundtrip_value, example_value);
    }

    #[test]
    fn deserializes_path_environment_pointer() {
        let path_pointer: EnvironmentPointer = serde_json::from_str(PATH_ENV_JSON).unwrap();
        assert_eq!(
            path_pointer,
            EnvironmentPointer::Path(PathPointer {
                name: EnvironmentName::from_str("name").unwrap(),
                version: Version::<1> {},
            })
        );
    }

    #[test]
    fn install_adds_new_package() {
        let test_packages = vec!["python".to_owned()];
        let pre_addition_toml = DUMMY_MANIFEST.parse::<Document>().unwrap();
        assert!(!contains_package(&pre_addition_toml, &test_packages[0]).unwrap());
        let (toml, changed) = insert_packages(DUMMY_MANIFEST, test_packages.iter().cloned())
            .expect("couldn't add package");
        assert!(changed, "manifest was changed by install");
        assert!(contains_package(&toml, &test_packages[0]).unwrap());
        eprintln!("{}", toml);
    }

    #[test]
    fn no_change_adding_existing_package() {
        let test_packages = vec!["hello".to_owned()];
        let pre_addition_toml = DUMMY_MANIFEST.parse::<Document>().unwrap();
        assert!(contains_package(&pre_addition_toml, &test_packages[0]).unwrap());
        let (_toml, changed) =
            insert_packages(DUMMY_MANIFEST, test_packages.iter().cloned()).unwrap();
        assert!(
            !changed,
            "manifest shouldn't be changed installing existing package"
        );
    }

    #[test]
    fn install_adds_install_table_when_missing() {
        let test_packages = vec!["foo".to_owned()];
        let (toml, changed) = insert_packages("", test_packages.iter().cloned()).unwrap();
        assert!(contains_package(&toml, &test_packages[0]).unwrap());
        assert!(changed, "manifest was changed by install");
    }

    #[test]
    fn install_error_when_manifest_malformed() {
        let test_packages = vec!["foo".to_owned()];
        let attempted_install = insert_packages(BAD_MANIFEST, test_packages.iter().cloned());
        assert!(matches!(
            attempted_install,
            Err(InstallError::MalformedManifest(_))
        ))
    }
}
