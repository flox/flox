use std::path::{Path, PathBuf};
use std::{fs, io};

use async_trait::async_trait;
use flox_types::catalog::{CatalogEntry, EnvCatalog, System};
use flox_types::version::Version;
use rnix::ast::{AttrSet, Expr};
use rowan::ast::AstNode;
use runix::command_line::{NixCommandLine, NixCommandLineRunError, NixCommandLineRunJsonError};
use runix::installable::FlakeAttribute;
use runix::store_path::StorePath;
use serde::{Deserialize, Serialize};
use thiserror::Error;
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
use crate::utils::rnix::{AttrSetExt, StrExt};

pub mod managed_environment;
pub mod path_environment;
pub mod remote_environment;

pub const CATALOG_JSON: &str = "catalog.json";
pub const DOT_FLOX: &str = ".flox";
pub const ENVIRONMENT_POINTER_FILENAME: &str = "env.json";
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
        packages: Vec<FloxPackage>,
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
    Managed(ManagedPointer),
    Path(PathPointer),
}

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
                io::ErrorKind::NotFound => Err(EnvironmentError2::DirectoryNotAnEnv)?,
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
}

/// Within a nix AST, find the first definition of an attribute set,
/// that is not part of a `let` expression or a where clause
fn find_attrs(mut expr: Expr) -> Result<AttrSet, ()> {
    loop {
        match expr {
            Expr::LetIn(let_in) => expr = let_in.body().unwrap(),
            Expr::With(with) => expr = with.body().unwrap(),

            Expr::AttrSet(attrset) => return Ok(attrset),
            _ => return Err(()),
        }
    }
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

pub enum ManifestContent {
    Unchanged,
    Changed(String),
}

/// insert packages into the content of a flox.nix file
///
/// TODO: At some point this should return Unchanged if the contents were not
/// changed, (e.g. the user tries to install a package that's already
/// installed).
fn flox_nix_content_with_new_packages(
    flox_nix_content: &impl AsRef<str>,
    packages: impl IntoIterator<Item = FloxPackage>,
) -> Result<ManifestContent, EnvironmentError2> {
    let packages = packages
        .into_iter()
        .map(|package| package.flox_nix_attribute().unwrap());

    let mut root = rnix::Root::parse(flox_nix_content.as_ref())
        .ok()
        .unwrap()
        .expr()
        .unwrap();

    if let Expr::Lambda(lambda) = root {
        root = lambda.body().unwrap();
    }

    let config_attrset = find_attrs(root.clone()).unwrap();
    #[allow(clippy::redundant_clone)] // required for rnix reasons, i think
    let mut edited = config_attrset.clone();

    for (path, version) in packages {
        let mut value = rnix::ast::AttrSet::new();
        if let Some(version) = version {
            value = value.insert_unchecked(
                ["version"],
                rnix::ast::Str::new(&version).syntax().to_owned(),
            );
        }

        let mut path_in_packages = vec!["packages".to_string()];
        path_in_packages.extend_from_slice(&path);
        edited = edited.insert_unchecked(path_in_packages, value.syntax().to_owned());
    }

    let green_tree = config_attrset
        .syntax()
        .replace_with(edited.syntax().green().into_owned());
    let new_content = nixpkgs_fmt::reformat_string(&green_tree.to_string());
    Ok(ManifestContent::Changed(new_content))
}

/// remove packages from the content of a flox.nix file
///
/// TODO: At some point this should return Unchanged (e.g. the user tries to
/// uninstall a package that's not installed).
fn flox_nix_content_with_packages_removed(
    flox_nix_content: &impl AsRef<str>,
    packages: impl IntoIterator<Item = FloxPackage>,
) -> Result<ManifestContent, EnvironmentError2> {
    let packages = packages
        .into_iter()
        .map(|package| package.flox_nix_attribute().unwrap());

    let mut root = rnix::Root::parse(flox_nix_content.as_ref())
        .ok()
        .unwrap()
        .expr()
        .unwrap();

    if let Expr::Lambda(lambda) = root {
        root = lambda.body().unwrap();
    }

    let config_attrset = find_attrs(root.clone()).unwrap();

    #[allow(clippy::redundant_clone)] // required for rnix reasons, i think
    let mut edited = config_attrset.clone().syntax().green().into_owned();

    for (path, _version) in packages {
        let mut path_in_packages = vec!["packages".to_string()];
        path_in_packages.extend_from_slice(&path);

        let index = config_attrset
            .find_by_path(&path_in_packages)
            .unwrap_or_else(|| panic!("path not found, {path_in_packages:?}"))
            .syntax()
            .index();
        edited = edited.remove_child(index - 2); // yikes
    }

    let green_tree = config_attrset.syntax().replace_with(edited);
    let new_content = nixpkgs_fmt::reformat_string(&green_tree.to_string());
    Ok(ManifestContent::Changed(new_content))
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
}
