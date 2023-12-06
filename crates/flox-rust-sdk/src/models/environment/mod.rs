use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs, io};

use async_trait::async_trait;
use flox_types::catalog::{CatalogEntry, EnvCatalog, System};
use flox_types::version::Version;
use log::debug;
use once_cell::sync::Lazy;
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
use super::manifest::{Manifest, TomlEditError};
use super::pkgdb_errors::PkgDbError;
use crate::flox::{EnvironmentRef, Flox};
use crate::utils::copy_file_without_permissions;
use crate::utils::errors::IoError;

pub mod generations;
pub mod managed_environment;
pub mod path_environment;
pub mod remote_environment;

pub const CATALOG_JSON: &str = "catalog.json";
// don't forget to update the man page
pub const DEFAULT_KEEP_GENERATIONS: usize = 10;
// don't forget to update the man page
pub const DEFAULT_MAX_AGE_DAYS: u32 = 90;

// Path to the executable that builds environments
pub static ENV_BUILDER_BIN: Lazy<String> =
    Lazy::new(|| env::var("ENV_BUILDER_BIN").unwrap_or(env!("ENV_BUILDER_BIN").to_string()));

pub const DOT_FLOX: &str = ".flox";
pub const ENVIRONMENT_POINTER_FILENAME: &str = "env.json";
pub const GLOBAL_MANIFEST_TEMPLATE: &str = env!("GLOBAL_MANIFEST_TEMPLATE");
pub const GLOBAL_MANIFEST_FILENAME: &str = "global-manifest.toml";
pub const MANIFEST_FILENAME: &str = "manifest.toml";
pub const LOCKFILE_FILENAME: &str = "manifest.lock";
pub const GCROOTS_DIR_NAME: &str = "run";
pub const ENV_DIR_NAME: &str = "env";
pub const FLOX_ENV_VAR: &str = "FLOX_ENV";
pub const FLOX_ACTIVE_ENVIRONMENTS_VAR: &str = "FLOX_ACTIVE_ENVIRONMENTS";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";

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
    async fn build(&mut self, flox: &Flox) -> Result<(), EnvironmentError2>;

    /// Install packages to the environment atomically
    async fn install(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError2>;

    /// Uninstall packages from the environment atomically
    async fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<String, EnvironmentError2>;

    /// Atomically edit this environment, ensuring that it still builds
    async fn edit(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<EditResult, EnvironmentError2>;

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
    async fn activation_path(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError2>;

    /// Directory containing .flox
    ///
    /// For anything internal, path should be used instead. `parent_path` is
    /// stored in FLOX_ACTIVE_ENVIRONMENTS and printed to users so that users
    /// don't have to see the trailing .flox
    /// TODO: figure out what to store for remote environments
    fn parent_path(&self) -> Result<PathBuf, EnvironmentError2>;

    /// Path to the environment definition file
    fn manifest_path(&self) -> PathBuf;

    /// Path to the lockfile. The path may not exist.
    fn lockfile_path(&self) -> PathBuf;

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
        let pointer_contents = match fs::read(&pointer_path) {
            Ok(contents) => contents,
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => {
                    debug!("couldn't find env.json at {}", pointer_path.display());
                    Err(EnvironmentError2::EnvNotFound)?
                },
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

/// Represents a .flox directory that contains an env.json.
///
/// An [UninitializedEnvironment] represents enough guarantees that we can treat
/// is as an environment. For example, we can prompt users asking if they want
/// to use it. Opening the environment with ManagedEnvironment::open or
/// PathEnvironment::open, however, could still fail.
pub struct UninitializedEnvironment {
    pub path: PathBuf,
    pub pointer: EnvironmentPointer,
}

impl UninitializedEnvironment {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EnvironmentError2> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            pointer: EnvironmentPointer::open(&path)?,
        })
    }
}

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq)]
pub struct LockedManifest(Value);
impl LockedManifest {
    /// Use pkgdb to lock a manifest
    pub fn lock_manifest(
        pkgdb: &Path,
        manifest_path: &Path,
        existing_lockfile_path: Option<&Path>,
        global_manifest_path: &Path,
    ) -> Result<Self, EnvironmentError2> {
        let canonical_manifest_path = manifest_path
            .canonicalize()
            .map_err(EnvironmentError2::OpenManifest)?;

        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd
            .args(["manifest", "lock"])
            .arg("--ga-registry")
            .arg("--global-manifest")
            .arg(global_manifest_path);
        if let Some(lf_path) = existing_lockfile_path {
            let canonical_lockfile_path = lf_path
                .canonicalize()
                .map_err(EnvironmentError2::BadLockfilePath)?;
            pkgdb_cmd.arg("--lockfile").arg(canonical_lockfile_path);
        }
        pkgdb_cmd.arg(canonical_manifest_path);

        debug!("locking manifest with command: {pkgdb_cmd:?}");
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
            let lockfile_json = serde_json::from_slice(&output.stdout)
                .map_err(EnvironmentError2::ParseLockfileJSON)?;
            Ok(lockfile_json)
        }
    }

    /// Build a locked manifest
    ///
    /// if a gcroot_out_link_path is provided,
    /// the environment will be linked to that path and a gcroot will be created
    pub fn build(
        &self,
        builder: &Path,
        gcroot_out_link_path: Option<&Path>,
    ) -> Result<PathBuf, EnvironmentError2> {
        let mut env_builder_cmd = Command::new(builder);
        env_builder_cmd.arg("build-env");
        env_builder_cmd.args(["--lockfile", &self.0.to_string()]);

        if let Some(gcroot_out_link_path) = gcroot_out_link_path {
            env_builder_cmd.args(["--out-link", &gcroot_out_link_path.to_string_lossy()]);
        }

        debug!("building environment with command: {env_builder_cmd:?}");

        let env_builder_output = env_builder_cmd
            .output()
            .map_err(EnvironmentError2::BuildEnvCall)?;

        if !env_builder_output.status.success() {
            let stderr = String::from_utf8_lossy(&env_builder_output.stderr).into_owned();
            return Err(EnvironmentError2::BuildEnv(stderr));
        }

        let stdout = String::from_utf8_lossy(&env_builder_output.stdout).into_owned();

        Ok(PathBuf::from(stdout.trim()))
    }
}
impl ToString for LockedManifest {
    fn to_string(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Debug)]
pub enum EditResult {
    /// The manifest was not modified.
    Unchanged,
    /// The manifest was modified, and the user needs to re-activate it.
    ReActivateRequired,
    /// The manifest was modified, but the user does not need to re-activate it.
    Success,
}

impl EditResult {
    pub fn new(old_manifest: &str, new_manifest: &str) -> Result<Self, EnvironmentError2> {
        if old_manifest == new_manifest {
            Ok(Self::Unchanged)
        } else {
            let old_manifest: Manifest =
                toml::from_str(old_manifest).map_err(EnvironmentError2::DeserializeManifest)?;
            let new_manifest: Manifest =
                toml::from_str(new_manifest).map_err(EnvironmentError2::DeserializeManifest)?;
            // TODO: some modifications to `install` currently require re-activation
            if old_manifest.hook != new_manifest.hook || old_manifest.vars != new_manifest.vars {
                Ok(Self::ReActivateRequired)
            } else {
                Ok(Self::Success)
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum EnvironmentError2 {
    #[error("ParseEnvRef")]
    ParseEnvRef(#[from] EnvironmentRefError),
    #[error("EmptyDotFlox")]
    EmptyDotFlox,
    #[error("DotFloxCanonicalize")]
    EnvCanonicalize(#[source] std::io::Error),
    #[error("ReadDotFlox")]
    ReadDotFlox(#[source] std::io::Error),
    #[error("ReadEnvDir")]
    ReadEnvDir(#[source] std::io::Error),
    #[error("MakeSandbox")]
    MakeSandbox(#[source] std::io::Error),
    #[error("DeleteEnvironment")]
    DeleteEnvironment(#[source] std::io::Error),
    #[error("DotFloxNotFound")]
    DotFloxNotFound,
    #[error("InitEnv")]
    InitEnv(#[source] std::io::Error),
    #[error("EnvNotFound")]
    EnvNotFound,
    #[error("EnvNotADirectory")]
    EnvNotADirectory,
    #[error("DirectoryNotAnEnv")]
    DirectoryNotAnEnv,
    #[error("EnvironmentExists")]
    EnvironmentExists,
    #[error("EvalCatalog")]
    EvalCatalog(#[source] NixCommandLineRunJsonError),
    #[error("ParseCatalog")]
    ParseCatalog(#[source] serde_json::Error),
    #[error("WriteCatalog")]
    WriteCatalog(#[source] std::io::Error),
    #[error("Build")]
    Build(#[source] NixCommandLineRunError),
    #[error("ReadManifest")]
    ReadManifest(#[source] std::io::Error),
    #[error("ReadEnvironmentMetadata")]
    ReadEnvironmentMetadata(#[source] std::io::Error),
    #[error("MakeTemporaryEnv")]
    MakeTemporaryEnv(#[source] std::io::Error),
    #[error("UpdateManifest")]
    UpdateManifest(#[source] std::io::Error),
    #[error("couldn't open manifest")]
    OpenManifest(#[source] std::io::Error),
    #[error("Activate")]
    Activate(#[source] NixCommandLineRunError),
    #[error("Prior transaction in progress. Delete {0} to discard.")]
    PriorTransaction(PathBuf),
    #[error("Failed to create backup for transaction")]
    BackupTransaction(#[source] std::io::Error),
    #[error("Failed to move modified environment into place")]
    Move(#[source] std::io::Error),
    #[error("Failed to abort transaction; backup could not be moved back into place")]
    AbortTransaction(#[source] std::io::Error),
    #[error("Failed to remove transaction backup")]
    RemoveBackup(#[source] std::io::Error),
    #[error("Failed to copy file")]
    CopyFile(#[source] IoError),
    #[error("Failed parsing contents of env.json file")]
    ParseEnvJson(#[source] serde_json::Error),
    #[error("Failed serializing contents of env.json file")]
    SerializeEnvJson(#[source] serde_json::Error),
    #[error("Failed write env.json file")]
    WriteEnvJson(#[source] std::io::Error),
    #[error(transparent)]
    ManagedEnvironment(#[from] ManagedEnvironmentError),
    #[error(transparent)]
    Install(#[from] TomlEditError),
    #[error("couldn't locate the manifest for this environment")]
    ManifestNotFound,
    #[error("failed to create GC roots directory")]
    CreateGcRootDir(#[source] std::io::Error),
    #[error("error building environment")]
    BuildEnvCall(#[source] std::io::Error),
    #[error("error building environment: {0}")]
    BuildEnv(String),
    #[error("provided lockfile path doesn't exist")]
    BadLockfilePath(#[source] std::io::Error),
    #[error("call to pkgdb failed")]
    PkgDbCall(#[source] std::io::Error),
    #[error("couldn't parse pkgdb error as JSON: {0}")]
    ParsePkgDbError(String),
    #[error("couldn't parse lockfile as JSON")]
    ParseLockfileJSON(#[source] serde_json::Error),
    #[error("couldn't parse nixpkgs rev as a string")]
    RevNotString,
    #[error("couldn't write new lockfile contents")]
    WriteLockfile(#[source] std::io::Error),
    #[error("locking manifest failed")]
    LockManifest(#[source] PkgDbError),
    #[error("couldn't create the global manifest")]
    InitGlobalManifest(#[source] std::io::Error),
    #[error("couldn't read global manifest template")]
    ReadGlobalManifestTemplate(#[source] std::io::Error),
    #[error("provided path couldn't be canonicalized: {path}")]
    CanonicalPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid internal state; couldn't remove last element from path: {0}")]
    InvalidPath(PathBuf),
    #[error("couldn't parse manifest: {0}")]
    DeserializeManifest(toml::de::Error),
    #[error("invalid .flox directory at {path}: {source}")]
    InvalidDotFlox {
        path: PathBuf,
        #[source]
        source: Box<EnvironmentError2>,
    },
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
    if !to.as_ref().exists() {
        std::fs::create_dir(to).unwrap();
    }

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

/// Initialize the global manifest if it doesn't exist already
pub fn init_global_manifest(global_manifest_path: &Path) -> Result<(), EnvironmentError2> {
    if !global_manifest_path.exists() {
        let global_manifest_template_contents =
            std::fs::read_to_string(Path::new(GLOBAL_MANIFEST_TEMPLATE))
                .map_err(EnvironmentError2::ReadGlobalManifestTemplate)?;
        std::fs::write(global_manifest_path, global_manifest_template_contents)
            .map_err(EnvironmentError2::InitGlobalManifest)?;
    }
    Ok(())
}

/// Returns the path to the global manifest
pub fn global_manifest_path(flox: &Flox) -> PathBuf {
    let path = flox.config_dir.join(GLOBAL_MANIFEST_FILENAME);
    debug!("global manifest path is {}", path.display());
    path
}

/// Searches for a `.flox` directory and attempts to parse env.json
///
/// The search first looks whether the current directory contains a `.flox` directory,
/// and if not, it searches upwards, stopping at the root directory.
pub fn find_dot_flox(
    initial_dir: &Path,
) -> Result<Option<UninitializedEnvironment>, EnvironmentError2> {
    let path = initial_dir
        .canonicalize()
        .map_err(|e| EnvironmentError2::CanonicalPath {
            path: initial_dir.to_path_buf(),
            source: e,
        })?;
    let mut tentative_dot_flox = path.join(DOT_FLOX);
    debug!(
        "looking for .flox: starting_path={}",
        tentative_dot_flox.display()
    );
    // Look for an immediate child named `.flox`
    if tentative_dot_flox.exists() {
        let pointer = UninitializedEnvironment::open(&path).map_err(|err| {
            EnvironmentError2::InvalidDotFlox {
                path: tentative_dot_flox.clone(),
                source: Box::new(err),
            }
        })?;
        debug!(".flox found: path={}", tentative_dot_flox.display());
        return Ok(Some(pointer));
    }
    // Search upwards for a .flox
    while let Some(grandparent) = tentative_dot_flox.parent().and_then(|p| p.parent()) {
        let grandparent_clone = grandparent.to_path_buf();
        tentative_dot_flox = grandparent.join(DOT_FLOX);
        if tentative_dot_flox.exists() {
            let pointer = UninitializedEnvironment::open(grandparent_clone).map_err(|err| {
                EnvironmentError2::InvalidDotFlox {
                    path: tentative_dot_flox.clone(),
                    source: Box::new(err),
                }
            })?;
            debug!(".flox found: path={}", tentative_dot_flox.display());
            return Ok(Some(pointer));
        }
    }
    Ok(None)
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

    // #[test]
    // fn discovers_immediate_child_dot_flox() {
    //     let temp_dir = tempfile::tempdir().unwrap();
    //     let actual_dot_flox = temp_dir.path().join(DOT_FLOX);
    //     std::fs::create_dir_all(&actual_dot_flox).unwrap();
    //     let found_dot_flox = find_dot_flox(temp_dir.path())
    //         .unwrap()
    //         .expect("expected to find dot flox");
    //     assert_eq!(found_dot_flox, actual_dot_flox.canonicalize().unwrap());
    // }

    // #[test]
    // fn discovers_existing_upwards_dot_flox() {
    //     let temp_dir = tempfile::tempdir().unwrap();
    //     let actual_dot_flox = temp_dir.path().join(DOT_FLOX);
    //     let start_path = actual_dot_flox.join("foo").join("bar");
    //     std::fs::create_dir_all(&start_path).unwrap();
    //     let found_dot_flox = find_dot_flox(&start_path)
    //         .unwrap()
    //         .expect("expected to find dot flox");
    //     assert_eq!(found_dot_flox, actual_dot_flox.canonicalize().unwrap());
    // }

    // #[test]
    // fn discovers_adjacent_dot_flox() {
    //     let temp_dir = tempfile::tempdir().unwrap();
    //     let actual_dot_flox = temp_dir.path().join(DOT_FLOX);
    //     std::fs::create_dir_all(&actual_dot_flox).unwrap();
    //     let found_dot_flox = find_dot_flox(&actual_dot_flox)
    //         .unwrap()
    //         .expect("expected to find dot flox");
    //     assert_eq!(found_dot_flox, actual_dot_flox.canonicalize().unwrap());
    // }

    // #[test]
    // fn no_error_on_discovering_nonexistent_dot_flox() {
    //     let temp_dir = tempfile::tempdir().unwrap();
    //     let start_path = temp_dir.path().join("foo").join("bar");
    //     std::fs::create_dir_all(&start_path).unwrap();
    //     let found_dot_flox = find_dot_flox(&start_path).unwrap();
    //     assert_eq!(found_dot_flox, None);
    // }

    #[test]
    fn error_when_discovering_dot_flox_in_nonexistent_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let start_path = temp_dir.path().join("foo").join("bar");
        let found_dot_flox = find_dot_flox(&start_path);
        assert!(found_dot_flox.is_err());
    }
}
