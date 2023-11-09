use std::collections::HashMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

use async_trait::async_trait;
use flox_types::catalog::{CatalogEntry, EnvCatalog, System};
use flox_types::version::Version;
use log::debug;
use runix::command_line::{NixCommandLine, NixCommandLineRunError, NixCommandLineRunJsonError};
use runix::installable::FlakeAttribute;
use runix::store_path::StorePath;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use walkdir::WalkDir;

use self::managed_environment::ManagedEnvironmentError;
use super::environment_ref::{EnvironmentName, EnvironmentOwner, EnvironmentRefError};
use super::flox_package::FloxTriple;
use super::manifest::TomlEditError;
use crate::flox::{EnvironmentRef, Flox};
use crate::utils::copy_file_without_permissions;
use crate::utils::errors::IoError;

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

// Path to the executable that builds environments
const BUILD_ENV_BIN: &'_ str = env!("BUILD_ENV_BIN");
const ENV_FROM_LOCKFILE_PATH: &str = env!("ENV_FROM_LOCKFILE_PATH");
const GLOBAL_MANIFEST_TEMPLATE: &str = env!("GLOBAL_MANIFEST_TEMPLATE");

pub enum InstalledPackage {
    Catalog(FloxTriple, CatalogEntry),
    FlakeAttribute(FlakeAttribute, CatalogEntry),
    StorePath(StorePath),
}

/// The result of an installation attempt that contains the new manifest contents
/// along with whether each package was already installed
#[derive(Debug)]
pub struct InstallationAttempt {
    pub new_manifest: Option<String>,
    pub already_installed: HashMap<String, bool>,
}

#[async_trait]
pub trait Environment {
    /// Build the environment and create a result link as gc-root
    async fn build(
        &mut self,
        nix: &NixCommandLine,
        system: &System,
    ) -> Result<(), EnvironmentError2>;

    /// Install packages to the environment atomically
    async fn install(
        &mut self,
        packages: Vec<String>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<InstallationAttempt, EnvironmentError2>;

    /// Uninstall packages from the environment atomically
    async fn uninstall(
        &mut self,
        packages: Vec<String>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<String, EnvironmentError2>;

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

    /// Return a path containing the built environment and its activation script.
    ///
    /// This should be a link to a store path so that it can be swapped
    /// dynamically, i.e. so that install/edit can modify the environment
    /// without requiring reactivation.
    async fn activation_path(
        &mut self,
        flox: &Flox,
        nix: &NixCommandLine,
    ) -> Result<PathBuf, EnvironmentError2>;

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
#[derive(Debug, Serialize, Clone, Deserialize, PartialEq)]
pub struct ManagedPointer {
    pub owner: EnvironmentOwner,
    pub name: EnvironmentName,
    version: Version<1>,
}

impl ManagedPointer {
    /// Create a new [ManagedPointer] with the given owner and name.
    pub fn new(owner: EnvironmentOwner, name: EnvironmentName) -> Self {
        Self {
            name,
            owner,
            version: Version::<1>,
        }
    }
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

impl From<EnvironmentRef> for ManagedPointer {
    fn from(value: EnvironmentRef) -> Self {
        Self::new(value.owner().clone(), value.name().clone())
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
    #[error("couldn't open manifest: {0}")]
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
    Install(#[from] TomlEditError),
    #[error("couldn't locate the manifest for this environment")]
    ManifestNotFound,
    #[error("failed to create GC roots directory: {0}")]
    CreateGcRootDir(std::io::Error),
    #[error("error building environment: {0}")]
    BuildEnvCall(std::io::Error),
    #[error("error building environment: {0}")]
    BuildEnv(String),
    #[error("provided lockfile path doesn't exist: {0}")]
    BadLockfilePath(std::io::Error),
    #[error("call to pkgdb failed: {0}")]
    PkgDbCall(std::io::Error),
    #[error("couldn't parse pkgdb error as JSON: {0}")]
    ParsePkgDbError(String),
    #[error("couldn't parse lockfile as JSON: {0}")]
    ParseLockfileJSON(serde_json::Error),
    #[error("couldn't parse nixpkgs rev as a string")]
    RevNotString,
    #[error("couldn't write new lockfile contents: {0}")]
    WriteLockfile(std::io::Error),
    #[error("locking manifest failed: {0}")]
    LockManifest(PkgDbError),
    #[error("couldn't create the global manifest: {0}")]
    InitGlobalManifest(std::io::Error),
    #[error("couldn't read global manifest template: {0}")]
    ReadGlobalManifestTemplate(std::io::Error),
}

/// A struct representing error messages coming from pkgdb
#[derive(Debug, Deserialize)]
pub struct PkgDbError {
    /// The exit code of pkgdb, can be used to programmatically determine
    /// the category of error.
    pub exit_code: u64,
    /// The generic message for this category of error.
    pub category_message: String,
    /// The more contextual message for the specific error that occurred.
    pub context_message: Option<String>,
    /// The underlying error message if an exception was caught.
    pub caught_message: Option<String>,
}

impl Display for PkgDbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.category_message)?;
        if let Some(ref context_message) = self.context_message {
            write!(f, ": {}", context_message)?;
        }
        if let Some(ref caught_message) = self.caught_message {
            write!(f, ": {}", caught_message)?;
        }
        Ok(())
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

/// Use pkgdb to lock a manifest
pub fn lock_manifest(
    pkgdb: &Path,
    manifest_path: &Path,
    existing_lockfile_path: Option<&Path>,
) -> Result<serde_json::Value, EnvironmentError2> {
    let canonical_manifest_path = manifest_path
        .canonicalize()
        .map_err(EnvironmentError2::OpenManifest)?;
    let mut pkgdb_cmd = Command::new(pkgdb);
    pkgdb_cmd
        .args(["manifest", "lock"])
        .arg(canonical_manifest_path);
    if let Some(lf_path) = existing_lockfile_path {
        let canonical_lockfile_path = lf_path
            .canonicalize()
            .map_err(EnvironmentError2::BadLockfilePath)?;
        pkgdb_cmd.arg(canonical_lockfile_path);
    }
    debug!(target: "posix", "locking manifest with command: {pkgdb_cmd:?}");
    let output = pkgdb_cmd.output().map_err(EnvironmentError2::PkgDbCall)?;
    // If command fails, try to parse stdout as a PkgDbError
    if !output.status.success() {
        if let Ok::<PkgDbError, _>(pkgdb_err) = serde_json::from_slice(&output.stdout) {
            Err(EnvironmentError2::LockManifest(pkgdb_err))
        } else {
            Err(EnvironmentError2::ParsePkgDbError(
                String::from_utf8_lossy(&output.stdout).to_string(),
            ))
        }
    // If command succeeds, try to parse stdout as JSON value
    } else {
        let lockfile_json: Value =
            serde_json::from_slice(&output.stdout).map_err(EnvironmentError2::ParseLockfileJSON)?;
        Ok(lockfile_json)
    }
}

/// Initialize the global manifest if it doesn't exist already
pub fn init_global_manifest(global_manifest_path: &Path) -> Result<(), EnvironmentError2> {
    if !global_manifest_path.exists() {
        let global_manifest_template_contents =
            std::fs::read_to_string(&Path::new(GLOBAL_MANIFEST_TEMPLATE))
                .map_err(EnvironmentError2::ReadGlobalManifestTemplate)?;
        std::fs::write(global_manifest_path, global_manifest_template_contents)
            .map_err(EnvironmentError2::InitGlobalManifest)?;
    }
    Ok(())
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
