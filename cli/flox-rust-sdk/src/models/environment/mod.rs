use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{fs, io};

use core_environment::UpgradeResult;
use log::debug;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use walkdir::WalkDir;

use self::managed_environment::ManagedEnvironmentError;
use self::remote_environment::RemoteEnvironmentError;
use super::container_builder::ContainerBuilder;
use super::env_registry::EnvRegistryError;
use super::environment_ref::{EnvironmentName, EnvironmentOwner};
use super::lockfile::{LockedManifest, LockedManifestError};
use super::manifest::{ManifestError, PackageToInstall, RawManifest, TomlEditError, TypedManifest};
use crate::data::{CanonicalPath, CanonicalizeError, Version};
use crate::flox::{Flox, Floxhub};
use crate::providers::git::{
    GitCommandDiscoverError,
    GitCommandProvider,
    GitDiscoverError,
    GitProvider,
};
use crate::utils::copy_file_without_permissions;

mod core_environment;
pub use core_environment::{test_helpers, CoreEnvironment, CoreEnvironmentError, EditResult};

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
pub const MANIFEST_FILENAME: &str = "manifest.toml";
pub const LOCKFILE_FILENAME: &str = "manifest.lock";
pub const GCROOTS_DIR_NAME: &str = "run";
pub const CACHE_DIR_NAME: &str = "cache";
pub const LIB_DIR_NAME: &str = "lib";
pub const LOG_DIR_NAME: &str = "log";
pub const ENV_DIR_NAME: &str = "env";
pub const FLOX_ENV_VAR: &str = "FLOX_ENV";

// The FLOX_* variables which follow are currently updated by the CLI as it
// activates new environments, and they are consequently *not* updated with
// manual invocations of the activation script. We want the activation script
// to eventually have feature parity with the CLI, so in future we will need
// to migrate this logic to the activation script itself. The only information
// known by the CLI that cannot be easily derived at runtime is the description
// text to be added to the prompt, and FLOX_ENV_DESCRIPTION_VAR was introduced
// to provide the means by which the CLI will be able to communicate this detail
// to the activation script.
pub const FLOX_ENV_DESCRIPTION_VAR: &str = "FLOX_ENV_DESCRIPTION";

pub const FLOX_ENV_CACHE_VAR: &str = "FLOX_ENV_CACHE";
pub const FLOX_ENV_PROJECT_VAR: &str = "FLOX_ENV_PROJECT";
pub const FLOX_ENV_DIRS_VAR: &str = "FLOX_ENV_DIRS";
pub const FLOX_ENV_LIB_DIRS_VAR: &str = "FLOX_ENV_LIB_DIRS";
pub const FLOX_ENV_LOG_DIR_VAR: &str = "_FLOX_ENV_LOG_DIR";
pub const FLOX_ACTIVE_ENVIRONMENTS_VAR: &str = "_FLOX_ACTIVE_ENVIRONMENTS";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";
pub const FLOX_SERVICES_SOCKET_VAR: &str = "_FLOX_SERVICES_SOCKET";

pub const N_HASH_CHARS: usize = 8;

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

/// Stores information about which of the manifest and lockfile need to be
/// migrated from v0 to v1
///
/// This struct should never be created if neither manifest nor lockfile need to
/// be migrated.
#[derive(Clone, Debug)]
pub struct MigrationInfo {
    /// The manifest is v0 and needs to be migrated to v1
    pub needs_manifest_migration: bool,
    /// The current lockfile is v0,
    /// or the manifest is v0 and the lockfile is v1.
    /// In either case, a migration requires changing the locked packages the
    /// user already has.
    pub needs_upgrade: bool,
    // Lets us skip re-reading the file
    raw_manifest: RawManifest,
}

pub trait Environment: Send {
    /// Create a container image from the environment
    fn build_container(&mut self, flox: &Flox) -> Result<ContainerBuilder, EnvironmentError>;

    /// Install packages to the environment atomically
    fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError>;

    /// Uninstall packages from the environment atomically
    fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<UninstallationAttempt, EnvironmentError>;

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError>;

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError>;

    /// Return the lockfile.
    ///
    /// Some implementations error if the lock does not already exist, while
    /// others call lock.
    fn lockfile(&mut self, flox: &Flox) -> Result<LockedManifest, EnvironmentError>;

    /// Extract the current content of the manifest
    ///
    /// Implementations may use process context from [Flox]
    /// to determine the current content of the manifest.
    fn manifest_contents(&self, flox: &Flox) -> Result<String, EnvironmentError>;

    /// Return the deserialized manifest
    fn manifest(&self, flox: &Flox) -> Result<TypedManifest, EnvironmentError>;

    /// Return a path containing the built environment and its activation script.
    ///
    /// This should be a link to a store path so that it can be swapped
    /// dynamically, i.e. so that install/edit can modify the environment
    /// without requiring reactivation.
    fn activation_path(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError>;

    /// Return a path that environment hooks should use to store transient data.
    ///
    /// The returned path will exist.
    fn cache_path(&self) -> Result<CanonicalPath, EnvironmentError>;

    /// Return a path that environment should use to store logs.
    ///
    /// The returned path will exist.
    fn log_path(&self) -> Result<CanonicalPath, EnvironmentError>;

    /// Return a path that should be used as the project root for environment hooks.
    fn project_path(&self) -> Result<PathBuf, EnvironmentError>;

    /// Directory containing .flox
    ///
    /// For anything internal, path should be used instead. `parent_path` is
    /// stored in _FLOX_ACTIVE_ENVIRONMENTS and printed to users so that users
    /// don't have to see the trailing .flox
    /// TODO: figure out what to store for remote environments
    fn parent_path(&self) -> Result<PathBuf, EnvironmentError>;

    /// Path to the environment's .flox directory
    fn dot_flox_path(&self) -> CanonicalPath;

    /// Path to the environment definition file
    ///
    /// Implementations may use process context from [Flox]
    /// to find or create a path to the environment definition file.
    ///
    /// [Environment::manifest_path] and [Environment::lockfile_path]
    /// may be located in different directories.
    fn manifest_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError>;

    /// Path to the lockfile. The path may not exist.
    ///
    /// Implementations may use process context from [Flox]
    /// to find or create a path to the environment definition file.
    ///
    /// [Environment::manifest_path] and [Environment::lockfile_path]
    /// may be located in different directories.
    fn lockfile_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError>;

    /// Returns the environment name
    fn name(&self) -> EnvironmentName;

    /// Delete the Environment
    fn delete(self, flox: &Flox) -> Result<(), EnvironmentError>
    where
        Self: Sized;

    /// Remove gc-roots
    // TODO consider renaming or removing - we might not support this for PathEnvironment
    fn delete_symlinks(&self) -> Result<bool, EnvironmentError> {
        Ok(false)
    }

    /// Possible actions depending on (version of manifest, version of lockfile)
    /// 0, None - manifest migration
    /// 0, 0 - manifest migration, upgrade
    /// 0, 1 - manifest migration, upgrade (unlikely state)
    /// 1, None - None
    /// 1, 0 - upgrade
    /// 1, 1 - None
    fn needs_migration_to_v1(
        &self,
        flox: &Flox,
    ) -> Result<Option<MigrationInfo>, EnvironmentError> {
        let raw_manifest = RawManifest::from_str(&self.manifest_contents(flox)?).map_err(|e| {
            EnvironmentError::Core(CoreEnvironmentError::ModifyToml(
                TomlEditError::ParseManifest(e),
            ))
        })?;
        let manifest = raw_manifest.to_typed().map_err(|e| {
            EnvironmentError::Core(CoreEnvironmentError::ModifyToml(
                TomlEditError::ParseManifest(e),
            ))
        })?;
        let needs_manifest_migration = match manifest {
            TypedManifest::Pkgdb(_) => true,
            TypedManifest::Catalog(_) => false,
        };

        let lockfile_path = self.lockfile_path(flox)?;
        let needs_upgrade = if let Ok(canonical_path) = CanonicalPath::new(lockfile_path) {
            // v0 manifest with any lockfile needs to be upgraded.
            // Having a v1 lockfile would be an unlikely state,
            // but just treat it as needing an upgrade.
            if needs_manifest_migration {
                true
            } else {
                let lockfile = LockedManifest::read_from_file(&canonical_path)
                    .map_err(EnvironmentError::LockedManifest)?;
                match lockfile {
                    LockedManifest::Pkgdb(_) => true,
                    LockedManifest::Catalog(_) => false,
                }
            }
        } else {
            // No existing lockfile, so no upgrade
            false
        };

        if !needs_manifest_migration && !needs_upgrade {
            return Ok(None);
        }

        Ok(Some(MigrationInfo {
            needs_manifest_migration,
            needs_upgrade,
            raw_manifest,
        }))
    }

    /// This will lock
    fn migrate_to_v1(
        &mut self,
        flox: &Flox,
        migration_info: MigrationInfo,
    ) -> Result<(), EnvironmentError>;

    fn services_socket_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError>;
}

/// A pointer to an environment, either managed or path.
/// This is used to determine the type of an environment at a given path.
/// See [EnvironmentPointer::open].
#[derive(
    Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, derive_more::From,
)]
#[serde(untagged)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub enum EnvironmentPointer {
    /// Identifies an environment whose source of truth lies outside of the project itself
    Managed(ManagedPointer),
    /// Identifies an environment whose source of truth lies inside the project
    Path(PathPointer),
}

/// The identifier for a project environment.
///
/// This is serialized to `env.json` inside the `.flox` directory
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
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
#[derive(Debug, Serialize, Clone, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct ManagedPointer {
    pub owner: EnvironmentOwner,
    pub name: EnvironmentName,
    #[cfg_attr(test, proptest(value = "crate::flox::DEFAULT_FLOXHUB_URL.clone()"))]
    pub floxhub_url: Url,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, proptest(value = "None"))]
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
    /// Attempt to read an environment pointer file ([ENVIRONMENT_POINTER_FILENAME])
    /// in the specified `.flox` directory.
    ///
    /// If the file is found and its contents can be deserialized,
    /// the function returns an [EnvironmentPointer] containing information about the environment.
    /// If reading or parsing fails, an [EnvironmentError] is returned.
    ///
    /// Use this method to determine the type of an environment at a given path.
    /// The result should be used to call the appropriate `open` method
    /// on either [path_environment::PathEnvironment] or [managed_environment::ManagedEnvironment].
    fn open(dot_flox_path: &CanonicalPath) -> Result<EnvironmentPointer, EnvironmentError> {
        let pointer_path = dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME);
        let pointer_contents = match fs::read(&pointer_path) {
            Ok(contents) => contents,
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => {
                    debug!("couldn't find env.json at {}", pointer_path.display());
                    Err(EnvironmentError::EnvPointerNotFound)?
                },
                _ => Err(EnvironmentError::ReadEnvironmentMetadata(err))?,
            },
        };

        serde_json::from_slice(&pointer_contents).map_err(EnvironmentError::ParseEnvJson)
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Eq, PartialOrd, Ord)]
pub struct DotFlox {
    pub path: PathBuf,
    pub pointer: EnvironmentPointer,
}

impl DotFlox {
    /// Open `<parent_path>/.flox` as a [DotFlox] directory
    ///
    /// This method attempts to find a `.flox` directory in the specified parent path,
    /// and open it as a [DotFlox] directory.
    /// If the directory is not found, an [EnvironmentError::DotFloxNotFound] is returned.
    pub fn open_in(parent_path: impl AsRef<Path>) -> Result<Self, EnvironmentError> {
        let dot_flox_path = parent_path.as_ref().join(DOT_FLOX);
        let dot_flox_path = CanonicalPath::new(&dot_flox_path)
            .map_err(|_| EnvironmentError::DotFloxNotFound(dot_flox_path))?;

        let pointer = EnvironmentPointer::open(&dot_flox_path)?;

        Ok(Self {
            path: dot_flox_path.to_path_buf(),
            pointer,
        })
    }
}

#[derive(Debug, Error)]
pub enum EnvironmentError {
    // todo: candidate for impl specific error
    // * only path and managed env are defined in .Flox
    // region: path env open
    /// The `.flox` directory was not found
    /// This error is thrown by calling [DotFlox::open_default_in]
    /// and callers in the `flox` crate if the `.flox` directory is not found.
    ///
    /// The error contains the path to the expected `.flox` directory,
    /// **including** the final `.flox` component.
    ///
    /// The [Display] implementation of this error displays the path to the parent directory.
    /// As for all practical purposes, we assume the final component to be `.flox`
    /// and communicate the parent directory to user when mentioning environment paths.
    #[error(
        "Did not find an environment in '{}'",
       .0.parent().map(PathBuf::from).as_ref().unwrap_or(.0).display()
    )]
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

    #[error("failed to create log directory")]
    CreateLogDir(#[source] std::io::Error),

    #[error("could not create temporary directory")]
    CreateTempDir(#[source] std::io::Error),

    #[error("could not get current directory")]
    GetCurrentDir(#[source] std::io::Error),

    #[error("failed to get canonical path for '.flox' directory")]
    CanonicalDotFlox(#[source] CanonicalizeError),

    #[error("failed to access the environment registry")]
    Registry(#[from] EnvRegistryError),

    #[error(transparent)]
    LockedManifest(LockedManifestError),

    #[error(transparent)]
    Canonicalize(CanonicalizeError),

    #[error("could not detect XDG_RUNTIME_DIR")]
    DetectRuntimeDir(#[source] xdg::BaseDirectoriesError),

    #[error("could not create services socket directory")]
    CreateServicesSocketDirectory(#[source] std::io::Error),

    #[error("path for services socket is too long: {0}")]
    ServicesSocketPathTooLong(PathBuf),

    #[error("corrupt environment; environment does not have a lockfile")]
    MissingLockfile,
}

#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    #[error(transparent)]
    PkgNotFound(#[from] ManifestError),
    #[error("'{pkg}' is a package in the group '{group}' with multiple packages")]
    NonEmptyNamedGroup { pkg: String, group: String },
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

/// Searches for a `.flox` directory and attempts to parse env.json
///
/// The search first looks whether the current directory contains a `.flox` directory.
/// If not, it checks if the current directory is contained by a git repo,
/// and if it is, it searches upwards, stopping at the repo toplevel.
pub fn find_dot_flox(initial_dir: &Path) -> Result<Option<DotFlox>, EnvironmentError> {
    let path = CanonicalPath::new(initial_dir).map_err(EnvironmentError::StartDiscoveryDir)?;

    let tentative_dot_flox = path.join(DOT_FLOX);
    debug!(
        "looking for .flox: starting_path={}",
        tentative_dot_flox.display()
    );
    // Look for an immediate child named `.flox`
    if tentative_dot_flox.exists() {
        let pointer = DotFlox::open_in(&path).map_err(|err| EnvironmentError::InvalidDotFlox {
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
        Err(e) => Err(EnvironmentError::DiscoverGitDirectory(e))?,
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
                DotFlox::open_in(ancestor).map_err(|err| EnvironmentError::InvalidDotFlox {
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

/// Returns the truncated hash of a [Path]
pub fn path_hash(p: impl AsRef<Path>) -> String {
    let mut chars = blake3::hash(p.as_ref().as_os_str().as_bytes()).to_hex();
    chars.truncate(N_HASH_CHARS);
    chars.to_string()
}

/// Return a path to the services socket given a unique identifier
///
/// Socket paths cannot exceed 104 characters on macOS
/// - TMPDIR will often have a long path, e.g.
///   /var/folders/8q/spckhr654cv4xrcv0fxsrlvc0000gn/T/nix-shell.vfDA8u
/// - /var/run is not writeable
///
/// So we use `flox.cache_dir.join("run")` which is typically
/// `~/.cache/flox/run`
///
/// On Linux use XDG_RUNTIME_DIR per
/// https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html
/// If unset, fallback to cache_dir like for macOS.
fn services_socket_path(id: &str, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
    if let Ok(path) = std::env::var(FLOX_SERVICES_SOCKET_VAR) {
        return Ok(PathBuf::from(path));
    }

    #[cfg(target_os = "macos")]
    let runtime_dir = None;
    #[cfg(target_os = "linux")]
    let runtime_dir = {
        let base_directories =
            xdg::BaseDirectories::new().map_err(EnvironmentError::DetectRuntimeDir)?;
        base_directories.get_runtime_directory().ok().cloned()
    };

    #[cfg(target_os = "macos")]
    let max_length = 104;
    #[cfg(target_os = "linux")]
    // 108 minus a null character
    let max_length = 107;

    let directory = match runtime_dir {
        Some(dir) => dir,
        None => {
            let fallback = flox.cache_dir.join("run");
            // We don't want to error if the directory already exists,
            // so use create_dir_all.
            std::fs::create_dir_all(&fallback)
                .map_err(EnvironmentError::CreateServicesSocketDirectory)?;
            fallback
        },
    };
    // Canonicalize so we error early if the path doesn't exist
    let canonicalized = CanonicalPath::new(directory).map_err(EnvironmentError::Canonicalize)?;

    let socket_path = canonicalized.join(format!("flox.{}.sock", id));

    if socket_path.as_os_str().len() > max_length {
        return Err(EnvironmentError::ServicesSocketPathTooLong(socket_path));
    }

    Ok(socket_path)
}

#[cfg(test)]
mod test {
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::PermissionsExt;
    use std::str::FromStr;
    use std::time::Duration;

    use once_cell::sync::Lazy;
    use path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_from_env_files,
    };
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::flox::DEFAULT_FLOXHUB_URL;
    use crate::providers::catalog::MANUALLY_GENERATED;
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
            Err(EnvironmentError::InvalidDotFlox { .. })
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
            path: temp_dir.path().join(DOT_FLOX).canonicalize().unwrap(),
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

        let found_environment = find_dot_flox(&start_path)
            .unwrap()
            .expect("expected to find dot flox");
        assert_eq!(found_environment, DotFlox {
            path: temp_dir.path().join(DOT_FLOX).canonicalize().unwrap(),
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
            path: temp_dir.path().join(DOT_FLOX).canonicalize().unwrap(),
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
            path: foo.join(DOT_FLOX).canonicalize().unwrap(),
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

    /// When manifest is v0 and lockfile does not exist, we need manifest
    /// migration but no upgrade
    #[test]
    fn needs_manifest_migration_0_none() {
        let (flox, _temp_dir_handle) = flox_instance();
        let environment = new_path_environment(&flox, "");
        assert!(matches!(
            environment.needs_migration_to_v1(&flox),
            Ok(Some(MigrationInfo {
                needs_manifest_migration: true,
                needs_upgrade: false,
                raw_manifest: _,
            }))
        ));
    }

    /// When manifest is v0 and lockfile is v0, we need manifest migration and
    /// upgrade
    #[test]
    fn needs_manifest_migration_0_0() {
        let (flox, _temp_dir_handle) = flox_instance();
        let environment =
            new_path_environment_from_env_files(&flox, MANUALLY_GENERATED.join("hello_v0"));
        assert!(matches!(
            environment.needs_migration_to_v1(&flox),
            Ok(Some(MigrationInfo {
                needs_manifest_migration: true,
                needs_upgrade: true,
                raw_manifest: _,
            }))
        ));
    }

    /// When manifest is v0 and lockfile is v1, we need manifest migration and
    /// upgrade
    #[test]
    fn needs_manifest_migration_0_1() {
        let (flox, _temp_dir_handle) = flox_instance();
        let environment = new_path_environment(&flox, "version = 1");
        let mut env_view = CoreEnvironment::new(environment.path.join(ENV_DIR_NAME));
        env_view.lock(&flox).unwrap();
        assert!(matches!(
            LockedManifest::read_from_file(
                &CanonicalPath::new(environment.lockfile_path(&flox).unwrap()).unwrap(),
            )
            .unwrap(),
            LockedManifest::Catalog(_)
        ));
        fs::write(environment.manifest_path(&flox).unwrap(), "").unwrap();
        assert!(matches!(
            environment.manifest(&flox).unwrap(),
            TypedManifest::Pkgdb(_),
        ));
        assert!(matches!(
            environment.needs_migration_to_v1(&flox),
            Ok(Some(MigrationInfo {
                needs_manifest_migration: true,
                needs_upgrade: true,
                raw_manifest: _,
            }))
        ));
    }

    /// When manifest is v1 and there's no lockfile, don't do anything
    #[test]
    fn needs_manifest_migration_1_none() {
        let (flox, _temp_dir_handle) = flox_instance();
        let environment = new_path_environment(&flox, "version = 1");
        assert!(environment.needs_migration_to_v1(&flox).unwrap().is_none());
    }

    /// When manifest is v1 and lockfile is v0, we need upgrade
    #[test]
    fn needs_manifest_migration_1_0() {
        let (flox, _temp_dir_handle) = flox_instance();
        let environment =
            new_path_environment_from_env_files(&flox, MANUALLY_GENERATED.join("hello_v0"));
        fs::write(environment.manifest_path(&flox).unwrap(), "version = 1").unwrap();
        assert!(matches!(
            environment.needs_migration_to_v1(&flox),
            Ok(Some(MigrationInfo {
                needs_manifest_migration: false,
                needs_upgrade: true,
                raw_manifest: _,
            }))
        ));
    }

    /// When manifest is v1 and lockfile is v0, we need upgrade
    #[test]
    fn needs_manifest_migration_1_1() {
        let (flox, _temp_dir_handle) = flox_instance();
        let environment = new_path_environment(&flox, "version = 1");
        let mut env_view = CoreEnvironment::new(environment.path.join(ENV_DIR_NAME));
        env_view.lock(&flox).unwrap();
        assert!(environment.needs_migration_to_v1(&flox).unwrap().is_none());
    }

    #[test]
    fn stable_path_hash() {
        // Ensure that running the path_hash function gives you the same results
        // with the same input e.g. doesn't depend on time, etc
        let (_flox, tmp_dir) = flox_instance();
        let path = tmp_dir.path().join("foo");
        std::fs::File::create(&path).unwrap();
        let path = CanonicalPath::new(path).unwrap();

        let hash1 = path_hash(&path);
        std::thread::sleep(Duration::from_millis(1_000));
        let hash2 = path_hash(&path);
        assert_eq!(hash1, hash2);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn services_socket_path_respects_xdg_runtime_dir() {
        let (flox, _temp_dir_handle) = flox_instance();
        // In reality XDG_RUNTIME_DIR would be something like `/run/user/1001`,
        // but that won't necessarily exist where this unit test is run.
        // We need a directory with group and others rights 00 otherwise
        // xdg::BaseDirectories errors.
        // And it needs to result in a path shorter than 107 characters.
        let tempdir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in("/tmp")
            .unwrap();
        let runtime_dir = tempdir.path();
        let socket_path = temp_env::with_var("XDG_RUNTIME_DIR", Some(&runtime_dir), || {
            services_socket_path("1", &flox)
        })
        .unwrap();
        assert_eq!(socket_path, runtime_dir.join("flox.1.sock"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn services_socket_path_falls_back_to_flox_cache() {
        let (flox, _temp_dir_handle) = flox_instance();
        let socket_path = temp_env::with_var("XDG_RUNTIME_DIR", None::<String>, || {
            services_socket_path("1", &flox)
        })
        .unwrap();
        assert_eq!(socket_path, flox.cache_dir.join("run/flox.1.sock"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn services_socket_path_errors_if_too_long() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        flox.cache_dir = flox.cache_dir.join("X".repeat(100));
        let err = temp_env::with_var("XDG_RUNTIME_DIR", None::<String>, || {
            services_socket_path("1", &flox)
        })
        .unwrap_err();
        assert!(matches!(
            err,
            EnvironmentError::ServicesSocketPathTooLong(_)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn services_socket_path_uses_flox_cache() {
        let (flox, _temp_dir_handle) = flox_instance();
        let socket_path = services_socket_path("1", &flox).unwrap();
        assert_eq!(
            socket_path,
            flox.cache_dir
                .canonicalize()
                .unwrap()
                .join("run/flox.1.sock")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn services_socket_path_errors_if_too_long() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        flox.cache_dir = flox.cache_dir.join("X".repeat(100));
        let err = services_socket_path("1", &flox).unwrap_err();
        assert!(matches!(
            err,
            EnvironmentError::ServicesSocketPathTooLong(_)
        ));
    }
}
