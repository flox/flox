use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

use log::debug;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use walkdir::WalkDir;

use self::managed_environment::ManagedEnvironmentError;
use self::remote_environment::RemoteEnvironmentError;
use super::container_builder::ContainerBuilder;
use super::environment_ref::{EnvironmentName, EnvironmentOwner};
use super::lockfile::LockedManifest;
use super::manifest::PackageToInstall;
use super::pkgdb::UpgradeResult;
use crate::data::Version;
use crate::flox::{Flox, Floxhub};
use crate::models::pkgdb::call_pkgdb;
use crate::providers::git::{
    GitCommandDiscoverError,
    GitCommandProvider,
    GitDiscoverError,
    GitProvider,
};
use crate::utils::copy_file_without_permissions;

mod core_environment;
pub use core_environment::{CoreEnvironmentError, EditResult};

pub mod generations;
pub mod managed_environment;
pub mod path_environment;
pub mod remote_environment;

pub const CATALOG_JSON: &str = "catalog.json";
// don't forget to update the man page
pub const DEFAULT_KEEP_GENERATIONS: usize = 10;
// don't forget to update the man page
pub const DEFAULT_MAX_AGE_DAYS: u32 = 90;

pub const DOT_FLOX: &str = ".flox";
pub const ENVIRONMENT_POINTER_FILENAME: &str = "env.json";
pub const GLOBAL_MANIFEST_TEMPLATE: &str = env!("GLOBAL_MANIFEST_TEMPLATE");
pub const GLOBAL_MANIFEST_FILENAME: &str = "global-manifest.toml";
pub const GLOBAL_MANIFEST_LOCKFILE_FILENAME: &str = "global-manifest.lock";
pub const MANIFEST_FILENAME: &str = "manifest.toml";
pub const LOCKFILE_FILENAME: &str = "manifest.lock";
pub const GCROOTS_DIR_NAME: &str = "run";
pub const CACHE_DIR_NAME: &str = "cache";
pub const ENV_DIR_NAME: &str = "env";
pub const FLOX_ENV_VAR: &str = "FLOX_ENV";
pub const FLOX_ENV_CACHE_VAR: &str = "FLOX_ENV_CACHE";
pub const FLOX_ENV_PROJECT_VAR: &str = "FLOX_ENV_PROJECT";
pub const FLOX_ENV_DIRS_VAR: &str = "FLOX_ENV_DIRS";
pub const FLOX_ENV_LIB_DIRS_VAR: &str = "FLOX_ENV_LIB_DIRS";
pub const FLOX_ACTIVE_ENVIRONMENTS_VAR: &str = "_FLOX_ACTIVE_ENVIRONMENTS";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";
pub const FLOX_PATH_PATCHED_VAR: &str = "FLOX_PATH_PATCHED";
pub const FLOX_SYSTEM_PLACEHOLDER: &str = "_FLOX_INIT_SYSTEM";
pub const FLOX_HOOK_PLACEHOLDER: &str = "_FLOX_INIT_HOOK";
pub const FLOX_INSTALL_PLACEHOLDER: &str = "_FLOX_INIT_INSTALL";

#[derive(Debug)]
pub struct UpdateResult {
    pub new_lockfile: LockedManifest,
    pub old_lockfile: Option<LockedManifest>,
    pub store_path: Option<PathBuf>,
}

/// A path that is guaranteed to be canonicalized
///
/// [`ManagedEnvironment`] uses this to refer to the path of its `.flox` directory.
/// [`ManagedEnvironment::encode`] is used to uniquely identify the environment
/// by encoding the canonicalized path.
/// This encoding is used to create a unique branch name in the floxmeta repository.
/// Thus, rather than canonicalizing the path every time we need to encode it,
/// we store the path as a [`CanonicalPath`].
#[derive(Debug, Clone, derive_more::Deref, derive_more::AsRef)]
#[deref(forward)]
#[as_ref(forward)]
pub struct CanonicalPath(PathBuf);

#[derive(Debug, Error)]
#[error("couldn't canonicalize path {path:?}: {err}")]
pub struct CanonicalizeError {
    pub path: PathBuf,
    #[source]
    pub err: std::io::Error,
}

impl CanonicalPath {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, CanonicalizeError> {
        let canonicalized = std::fs::canonicalize(&path).map_err(|e| CanonicalizeError {
            path: path.as_ref().to_path_buf(),
            err: e,
        })?;
        Ok(Self(canonicalized))
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

/// The result of an installation attempt that contains the new manifest contents
/// along with whether each package was already installed
#[derive(Debug)]
pub struct InstallationAttempt {
    pub new_manifest: Option<String>,
    pub already_installed: HashMap<String, bool>,
    /// The store path of environment that was built to validate the install.
    /// This is used as an optimization to skip builds that we've already done.
    pub store_path: Option<PathBuf>,
}

/// The result of an uninstallation attempt
#[derive(Debug)]
pub struct UninstallationAttempt {
    pub new_manifest: Option<String>,
    /// The store path of environment that was built to validate the uninstall.
    /// This is used as an optimization to skip builds that we've already done.
    pub store_path: Option<PathBuf>,
}

pub trait Environment: Send {
    /// Build the environment and create a result link as gc-root
    fn build(&mut self, flox: &Flox) -> Result<(), EnvironmentError2>;

    /// Resolve the environment and return the lockfile
    fn lock(&mut self, flox: &Flox) -> Result<LockedManifest, EnvironmentError2>;

    /// Create a container image from the environment
    fn build_container(&mut self, flox: &Flox) -> Result<ContainerBuilder, EnvironmentError2>;

    /// Install packages to the environment atomically
    fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError2>;

    /// Uninstall packages from the environment atomically
    fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<UninstallationAttempt, EnvironmentError2>;

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError2>;

    /// Atomically update this environment's inputs
    fn update(
        &mut self,
        flox: &Flox,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, EnvironmentError2>;

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[String],
    ) -> Result<UpgradeResult, EnvironmentError2>;

    /// Extract the current content of the manifest
    ///
    /// Implementations may use process context from [Flox]
    /// to determine the current content of the manifest.
    fn manifest_content(&self, flox: &Flox) -> Result<String, EnvironmentError2>;

    /// Return a path containing the built environment and its activation script.
    ///
    /// This should be a link to a store path so that it can be swapped
    /// dynamically, i.e. so that install/edit can modify the environment
    /// without requiring reactivation.
    fn activation_path(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError2>;

    /// Return a path that environment hooks should use to store transient data.
    fn cache_path(&self) -> Result<PathBuf, EnvironmentError2>;

    /// Return a path that should be used as the project root for environment hooks.
    fn project_path(&self) -> Result<PathBuf, EnvironmentError2>;

    /// Directory containing .flox
    ///
    /// For anything internal, path should be used instead. `parent_path` is
    /// stored in _FLOX_ACTIVE_ENVIRONMENTS and printed to users so that users
    /// don't have to see the trailing .flox
    /// TODO: figure out what to store for remote environments
    fn parent_path(&self) -> Result<PathBuf, EnvironmentError2>;

    /// Path to the environment definition file
    ///
    /// Implementations may use process context from [Flox]
    /// to find or create a path to the environment definition file.
    ///
    /// [Environment::manifest_path] and [Environment::lockfile_path]
    /// may be located in different directories.
    fn manifest_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError2>;

    /// Path to the lockfile. The path may not exist.
    ///
    /// Implementations may use process context from [Flox]
    /// to find or create a path to the environment definition file.
    ///
    /// [Environment::manifest_path] and [Environment::lockfile_path]
    /// may be located in different directories.
    fn lockfile_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError2>;

    /// Returns the environment name
    fn name(&self) -> EnvironmentName;

    /// Delete the Environment
    fn delete(self, flox: &Flox) -> Result<(), EnvironmentError2>
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, derive_more::From)]
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
    pub floxhub_url: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floxhub_git_url_override: Option<Url>,
    version: Version<1>,
}

impl ManagedPointer {
    /// Create a new [ManagedPointer] with the given owner and name.
    pub fn new(owner: EnvironmentOwner, name: EnvironmentName, floxhub: &Floxhub) -> Self {
        Self {
            name,
            owner,
            floxhub_url: floxhub.base_url().clone(),
            floxhub_git_url_override: floxhub.git_url_override().cloned(),
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
        if !dot_flox_path.exists() {
            debug!("couldn't find .flox at {}", dot_flox_path.display());
            Err(EnvironmentError2::DotFloxNotFound(
                dot_flox_path
                    .parent()
                    .unwrap_or(&dot_flox_path)
                    .to_path_buf(),
            ))?
        }
        let pointer_path = dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME);
        let pointer_contents = match fs::read(&pointer_path) {
            Ok(contents) => contents,
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => {
                    debug!("couldn't find env.json at {}", pointer_path.display());
                    Err(EnvironmentError2::EnvPointerNotFound)?
                },
                _ => Err(EnvironmentError2::ReadEnvironmentMetadata(err))?,
            },
        };

        serde_json::from_slice(&pointer_contents).map_err(EnvironmentError2::ParseEnvJson)
    }

    pub fn name(&self) -> &EnvironmentName {
        match self {
            EnvironmentPointer::Managed(pointer) => &pointer.name,
            EnvironmentPointer::Path(pointer) => &pointer.name,
        }
    }

    pub fn owner(&self) -> Option<&EnvironmentOwner> {
        match self {
            EnvironmentPointer::Managed(pointer) => Some(&pointer.owner),
            EnvironmentPointer::Path(_) => None,
        }
    }
}

/// Represents a `.flox` directory that contains an `env.json`.
///
/// A [DotFlox] represents a fully qualified reference to open
/// either a [PathEnvironment] or [ManagedEnvironment].
/// It is additionally used to provide more precise targets for the interactive
/// selection of environments.
///
/// However, this type does not perform any validation of the referenced environment.
/// Opening the environment with [ManagedEnvironment::open] or
/// [PathEnvironment::open], could still fail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DotFlox {
    pub path: PathBuf,
    pub pointer: EnvironmentPointer,
}

impl DotFlox {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EnvironmentError2> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            pointer: EnvironmentPointer::open(&path)?,
        })
    }
}

#[derive(Debug, Error)]
pub enum EnvironmentError2 {
    // todo: candidate for impl specific error
    // * only path and managed env are defined in .Flox
    // region: path env open
    #[error("Did not find an environment in '{0}'")]
    DotFloxNotFound(PathBuf),

    #[error("could not locate the manifest for this environment")]
    ManifestNotFound,

    // endregion

    // todo: candidate for impl specific error
    // * only path env implements init
    // region: path env init
    // todo: split up
    // * three distinct errors map to this
    #[error("could not initialize environment")]
    InitEnv(#[source] std::io::Error),
    /// .flox exists but .flox/env does not
    #[error("could not find environment definition directory")]
    EnvDirNotFound,
    #[error("could not find environment pointer file")]
    EnvPointerNotFound,
    #[error("an environment already exists at {0:?}")]
    EnvironmentExists(PathBuf),
    #[error("could not write .gitignore file")]
    WriteGitignore(#[source] std::io::Error),
    // endregion

    // todo: move pointer related errors somewhere else?
    // * not relevant to environment _instances_
    // region: pointer
    #[error("could not read env.json file")]
    ReadEnvironmentMetadata(#[source] std::io::Error),
    #[error("Failed parsing contents of env.json file")]
    ParseEnvJson(#[source] serde_json::Error),
    #[error("Failed serializing contents of env.json file")]
    SerializeEnvJson(#[source] serde_json::Error),
    #[error("Failed write env.json file")]
    WriteEnvJson(#[source] std::io::Error),
    // endregion

    // region: global manifest
    #[error("couldn't create the global manifest")]
    InitGlobalManifest(#[source] std::io::Error),

    #[error("couldn't read global manifest template")]
    ReadGlobalManifestTemplate(#[source] std::io::Error),
    // endregion

    // region: find_dot_flox
    // todo: extract and reuse in other places where we need to canonicalize a path
    #[error(transparent)]
    StartDiscoveryDir(CanonicalizeError),

    // todo: reword?
    // * only occurs if "`.flox`" is `/`
    #[error("invalid internal state; couldn't remove last element from path: {0}")]
    InvalidPath(PathBuf),

    #[error("invalid .flox directory at {path}")]
    InvalidDotFlox {
        path: PathBuf,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("error checking if in a git repo")]
    DiscoverGitDirectory(#[source] GitCommandDiscoverError),
    // endregion
    #[error(transparent)]
    Core(#[from] CoreEnvironmentError),

    #[error(transparent)]
    ManagedEnvironment(#[from] ManagedEnvironmentError),

    #[error(transparent)]
    RemoteEnvironment(#[from] RemoteEnvironmentError),

    #[error("could not delete environment")]
    DeleteEnvironment(#[source] std::io::Error),

    #[error("could not read manifest")]
    ReadManifest(#[source] std::io::Error),
    #[error("couldn't write manifest")]
    WriteManifest(#[source] std::io::Error),

    #[error("failed to create GC roots directory")]
    CreateGcRootDir(#[source] std::io::Error),

    #[error("failed to create cache directory")]
    CreateCacheDir(#[source] std::io::Error),

    #[error("could not create temporary directory")]
    CreateTempDir(#[source] std::io::Error),

    #[error("could not get current directory")]
    GetCurrentDir(#[source] std::io::Error),
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

/// Returns the path to the global manifest's lockfile
pub fn global_manifest_lockfile_path(flox: &Flox) -> PathBuf {
    let path = flox.config_dir.join(GLOBAL_MANIFEST_LOCKFILE_FILENAME);
    debug!("global manifest lockfile path is {}", path.display());
    path
}

/// Searches for a `.flox` directory and attempts to parse env.json
///
/// The search first looks whether the current directory contains a `.flox` directory,
/// and if not, it searches upwards, stopping at the root directory.
pub fn find_dot_flox(initial_dir: &Path) -> Result<Option<DotFlox>, EnvironmentError2> {
    let path = CanonicalPath::new(initial_dir).map_err(EnvironmentError2::StartDiscoveryDir)?;

    let tentative_dot_flox = path.join(DOT_FLOX);
    debug!(
        "looking for .flox: starting_path={}",
        tentative_dot_flox.display()
    );
    // Look for an immediate child named `.flox`
    if tentative_dot_flox.exists() {
        let pointer = DotFlox::open(&path).map_err(|err| EnvironmentError2::InvalidDotFlox {
            path: tentative_dot_flox.clone(),
            source: Box::new(err),
        })?;
        debug!(".flox found: path={}", tentative_dot_flox.display());
        return Ok(Some(pointer));
    }

    // Check if we're in a git repo.
    let toplevel = match GitCommandProvider::discover(&path) {
        Ok(repo) if repo.workdir().is_some() => repo.workdir().unwrap().to_owned(),
        Ok(_) => return Ok(None),
        // Assume we're not in a git repo.
        // TODO: could not_found() correspond to some other error?
        Err(e) if e.not_found() => return Ok(None),
        Err(e) => Err(EnvironmentError2::DiscoverGitDirectory(e))?,
    };

    // We already checked the immediate child.
    for ancestor in path.ancestors().skip(1) {
        // If we're above the git repo, return None.
        // ancestor and toplevel have both been canonicalized.
        if !ancestor.starts_with(&toplevel) {
            debug!("git boundary reached: path={}", ancestor.display());
            return Ok(None);
        }
        let tentative_dot_flox = ancestor.join(DOT_FLOX);
        debug!("looking for .flox: path={}", tentative_dot_flox.display());

        if tentative_dot_flox.exists() {
            let pointer =
                DotFlox::open(ancestor).map_err(|err| EnvironmentError2::InvalidDotFlox {
                    path: ancestor.to_path_buf(),
                    source: Box::new(err),
                })?;
            debug!(".flox found: path={}", tentative_dot_flox.display());
            return Ok(Some(pointer));
        }
    }
    Ok(None)
}

/// Directory containing nix gc roots for (previous) builds of environments of a given owner
pub(super) fn gcroots_dir(flox: &Flox, owner: &EnvironmentOwner) -> PathBuf {
    flox.cache_dir.join(GCROOTS_DIR_NAME).join(owner.as_str())
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use once_cell::sync::Lazy;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::flox::DEFAULT_FLOXHUB_URL;
    use crate::providers::git::GitProvider;

    const MANAGED_ENV_JSON: &'_ str = r#"{
        "name": "name",
        "owner": "owner",
        "floxhub_url": "https://hub.flox.dev/",
        "version": 1
    }"#;

    const PATH_ENV_JSON: &'_ str = r#"{
        "name": "name",
        "version": 1
    }"#;

    static MANAGED_ENV_POINTER: Lazy<EnvironmentPointer> = Lazy::new(|| {
        EnvironmentPointer::Managed(ManagedPointer {
            name: EnvironmentName::from_str("name").unwrap(),
            owner: EnvironmentOwner::from_str("owner").unwrap(),
            floxhub_url: DEFAULT_FLOXHUB_URL.clone(),
            floxhub_git_url_override: None,
            version: Version::<1> {},
        })
    });

    #[test]
    fn serializes_managed_environment_pointer() {
        let managed_pointer = EnvironmentPointer::Managed(ManagedPointer {
            name: EnvironmentName::from_str("name").unwrap(),
            owner: EnvironmentOwner::from_str("owner").unwrap(),
            floxhub_url: DEFAULT_FLOXHUB_URL.clone(),
            floxhub_git_url_override: None,
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
        assert_eq!(managed_pointer, *MANAGED_ENV_POINTER);
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
    fn errors_immediate_child_invalid() {
        let temp_dir = tempfile::tempdir().unwrap();
        let actual_dot_flox = temp_dir.path().join(DOT_FLOX);
        std::fs::create_dir_all(actual_dot_flox).unwrap();
        assert!(matches!(
            find_dot_flox(temp_dir.path()),
            Err(EnvironmentError2::InvalidDotFlox { .. })
        ))
    }

    #[test]
    fn discovers_immediate_child_dot_flox() {
        let temp_dir = tempfile::tempdir().unwrap();
        let actual_dot_flox = temp_dir.path().join(DOT_FLOX);
        std::fs::create_dir_all(&actual_dot_flox).unwrap();

        fs::write(
            actual_dot_flox.join(ENVIRONMENT_POINTER_FILENAME),
            serde_json::to_string_pretty(&*MANAGED_ENV_POINTER).unwrap(),
        )
        .unwrap();

        let found_environment = find_dot_flox(temp_dir.path())
            .unwrap()
            .expect("expected to find dot flox");
        assert_eq!(found_environment, DotFlox {
            path: temp_dir.path().canonicalize().unwrap(),
            pointer: (*MANAGED_ENV_POINTER).clone()
        });
    }

    /// An environment is found upwards, but only if it is within a git repo.
    #[test]
    fn discovers_existing_upwards_dot_flox() {
        let temp_dir = tempfile::tempdir().unwrap();
        let actual_dot_flox = temp_dir.path().join(DOT_FLOX);
        let start_path = actual_dot_flox.join("foo").join("bar");
        std::fs::create_dir_all(&start_path).unwrap();
        fs::write(
            actual_dot_flox.join(ENVIRONMENT_POINTER_FILENAME),
            serde_json::to_string_pretty(&*MANAGED_ENV_POINTER).unwrap(),
        )
        .unwrap();

        let found_environment = find_dot_flox(&start_path).unwrap();
        assert_eq!(found_environment, None);

        GitCommandProvider::init(temp_dir.path(), false).unwrap();

        let found_environment = find_dot_flox(temp_dir.path())
            .unwrap()
            .expect("expected to find dot flox");
        assert_eq!(found_environment, DotFlox {
            path: temp_dir.path().canonicalize().unwrap(),
            pointer: (*MANAGED_ENV_POINTER).clone()
        });
    }

    /// An environment is found upwards and adjacent, but only if it is within
    /// a git repo.
    ///
    /// .
    /// ├── .flox
    /// │   └── env.json
    /// └── foo
    ///     └── bar
    #[test]
    fn discovers_upwards_adjacent_dot_flox() {
        let temp_dir = tempfile::tempdir().unwrap();
        let actual_dot_flox = temp_dir.path().join(DOT_FLOX);
        std::fs::create_dir_all(&actual_dot_flox).unwrap();
        let start_path = temp_dir.path().join("foo").join("bar");
        std::fs::create_dir_all(&start_path).unwrap();
        fs::write(
            actual_dot_flox.join(ENVIRONMENT_POINTER_FILENAME),
            serde_json::to_string_pretty(&*MANAGED_ENV_POINTER).unwrap(),
        )
        .unwrap();

        let found_environment = find_dot_flox(&start_path).unwrap();
        assert_eq!(found_environment, None);

        GitCommandProvider::init(temp_dir.path(), false).unwrap();

        let found_environment = find_dot_flox(&start_path)
            .unwrap()
            .expect("expected to find dot flox");
        assert_eq!(found_environment, DotFlox {
            path: temp_dir.path().canonicalize().unwrap(),
            pointer: (*MANAGED_ENV_POINTER).clone()
        });
    }

    /// An environment is found upwards and adjacent when it is a subdirectory
    /// of a git repo.
    ///
    /// .
    /// ├── .git
    /// └── foo
    ///     ├── .flox
    ///     │   └── env.json
    ///     └── bar
    #[test]
    fn discovers_upwards_git_subdirectory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();
        let foo = path.join("foo");
        let actual_dot_flox = foo.join(DOT_FLOX);
        std::fs::create_dir_all(&actual_dot_flox).unwrap();
        let start_path = foo.join("bar");
        std::fs::create_dir_all(&start_path).unwrap();
        fs::write(
            actual_dot_flox.join(ENVIRONMENT_POINTER_FILENAME),
            serde_json::to_string_pretty(&*MANAGED_ENV_POINTER).unwrap(),
        )
        .unwrap();

        let found_environment = find_dot_flox(&start_path).unwrap();
        assert_eq!(found_environment, None);

        GitCommandProvider::init(path, false).unwrap();

        let found_environment = find_dot_flox(&start_path)
            .unwrap()
            .expect("expected to find dot flox");
        assert_eq!(found_environment, DotFlox {
            path: foo.canonicalize().unwrap(),
            pointer: (*MANAGED_ENV_POINTER).clone()
        });
    }

    /// An environment above a git repo is not found.
    ///
    /// .
    /// ├── .flox
    /// │   └── env.json
    /// └── foo
    ///     ├── .git
    ///     └── bar
    #[test]
    fn does_not_discover_above_git_repo() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path();
        let foo = path.join("foo");
        let actual_dot_flox = path.join(DOT_FLOX);
        std::fs::create_dir_all(&actual_dot_flox).unwrap();
        let start_path = foo.join("bar");
        std::fs::create_dir_all(&start_path).unwrap();
        fs::write(
            actual_dot_flox.join(ENVIRONMENT_POINTER_FILENAME),
            serde_json::to_string_pretty(&*MANAGED_ENV_POINTER).unwrap(),
        )
        .unwrap();

        GitCommandProvider::init(foo, false).unwrap();

        let found_environment = find_dot_flox(&start_path).unwrap();
        assert_eq!(found_environment, None);
    }

    #[test]
    fn no_error_on_discovering_nonexistent_dot_flox() {
        let temp_dir = tempfile::tempdir().unwrap();
        let start_path = temp_dir.path().join("foo").join("bar");
        std::fs::create_dir_all(&start_path).unwrap();
        let found_environment = find_dot_flox(&start_path).unwrap();
        assert_eq!(found_environment, None);
    }

    #[test]
    fn error_when_discovering_dot_flox_in_nonexistent_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let start_path = temp_dir.path().join("foo").join("bar");
        let found_environment = find_dot_flox(&start_path);
        assert!(found_environment.is_err());
    }
}
