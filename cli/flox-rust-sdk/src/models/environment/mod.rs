use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::{fs, io};

use enum_dispatch::enum_dispatch;
pub use flox_core::{Version, path_hash};
use indoc::formatdoc;
use managed_environment::ManagedEnvironment;
use path_environment::PathEnvironment;
use remote_environment::RemoteEnvironment;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;
use url::{ParseError, Url};
use walkdir::WalkDir;

use self::managed_environment::ManagedEnvironmentError;
use self::remote_environment::RemoteEnvironmentError;
use super::env_registry::EnvRegistryError;
use super::environment_ref::{EnvironmentName, EnvironmentOwner};
use super::lockfile::{LockResult, LockedInclude, Lockfile, RecoverableMergeError, ResolveError};
use super::manifest::raw::PackageToInstall;
use super::manifest::typed::{ActivateMode, ManifestError};
use crate::data::{CanonicalPath, CanonicalizeError, System};
use crate::flox::{Flox, Floxhub};
use crate::models::environment::generations::GenerationsEnvironment;
use crate::providers::auth::AuthError;
use crate::providers::buildenv::BuildEnvOutputs;
use crate::providers::git::{
    GitCommandDiscoverError,
    GitCommandProvider,
    GitDiscoverError,
    GitProvider,
};
use crate::utils::copy_file_without_permissions;

mod core_environment;
pub use core_environment::{
    CoreEnvironment,
    CoreEnvironmentError,
    EditResult,
    SingleSystemUpgradeDiff,
    UpgradeResult,
    test_helpers,
};

pub mod fetcher;
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

// The FLOX_* variables which follow are currently updated by the CLI as it
// activates new environments, and they are consequently *not* updated with
// manual invocations of the activation script. We want the activation script
// to eventually have feature parity with the CLI, so in future we will need
// to migrate this logic to the activation script itself.

pub const FLOX_ENV_LOG_DIR_VAR: &str = "_FLOX_ENV_LOG_DIR";
pub const FLOX_ACTIVE_ENVIRONMENTS_VAR: &str = "_FLOX_ACTIVE_ENVIRONMENTS";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";
/// This variable is used to communicate what socket to use to the activate
/// script.
pub const FLOX_SERVICES_SOCKET_VAR: &str = "_FLOX_SERVICES_SOCKET";
/// This variable is used in tests to override what path to use for the socket.
pub const FLOX_SERVICES_SOCKET_OVERRIDE_VAR: &str = "_FLOX_SERVICES_SOCKET_OVERRIDE";

pub use flox_core::N_HASH_CHARS;

/// The result of an installation attempt that contains the new manifest contents
/// along with whether each package was already installed
#[derive(Debug)]
pub struct InstallationAttempt {
    pub new_manifest: Option<String>,
    pub already_installed: HashMap<String, bool>,
    /// The store paths of environment that was built to validate the install.
    /// This is used as an optimization to skip builds that we've already done.
    pub built_environments: Option<BuildEnvOutputs>,
}

/// The result of an uninstallation attempt
#[derive(Debug)]
pub struct UninstallationAttempt {
    pub new_manifest: Option<String>,
    /// Packages that were requested to be uninstalled but are stilled provided
    /// by included environments.
    pub still_included: HashMap<String, LockedInclude>,
    /// The store path of environment that was built to validate the uninstall.
    /// This is used as an optimization to skip builds that we've already done.
    pub built_environment_store_paths: Option<BuildEnvOutputs>,
}

#[enum_dispatch]
pub trait Environment: Send {
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

    /// Upgrade packages in this environment without modifying the environment on disk.
    fn dry_upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError>;

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError>;

    /// Upgrade environment with latest changes to included environments.
    fn include_upgrade(
        &mut self,
        flox: &Flox,
        to_upgrade: Vec<String>,
    ) -> Result<UpgradeResult, EnvironmentError>;

    /// Return the lockfile.
    ///
    /// Some implementations error if the lock does not already exist, while
    /// others call lock.
    fn lockfile(&mut self, flox: &Flox) -> Result<LockResult, EnvironmentError>;

    /// Extract the current content of the manifest
    ///
    /// Implementations may use process context from [Flox]
    /// to determine the current content of the manifest.
    fn manifest_contents(&self, flox: &Flox) -> Result<String, EnvironmentError>;

    /// Return the path to rendered environment in the Nix store.
    ///
    /// This should be a link to a store path so that it can be swapped
    /// dynamically, i.e. so that install/edit can modify the environment
    /// without requiring reactivation.
    fn rendered_env_links(
        &mut self,
        flox: &Flox,
    ) -> Result<RenderedEnvironmentLinks, EnvironmentError>;

    /// Build the environment and return the built store paths
    /// for the development and runtime variants,
    /// as well as runtime environments of the manifest builds defined in this environment.
    ///
    /// This does not link the environment, but may lock the environment, if necessary.
    fn build(&mut self, flox: &Flox) -> Result<BuildEnvOutputs, EnvironmentError>;

    /// Return a path to store transient data,
    /// such as temporary files created by the environment hooks or the environment itself,
    /// including reproducible data about the environment.
    ///
    /// The returned path will exist.
    fn cache_path(&self) -> Result<CanonicalPath, EnvironmentError>;

    /// Return a path that environment should use to store logs.
    ///
    /// New log file patterns need to be added to `flox-watchdog` for garbage collection.
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

    /// Returns the lockfile if it already exists.
    fn existing_lockfile(&self, flox: &Flox) -> Result<Option<Lockfile>, EnvironmentError>;

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

    fn services_socket_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError>;
}

/// The various ways in which an environment can be referred to
#[enum_dispatch(Environment)]
#[derive(Debug)]
pub enum ConcreteEnvironment {
    /// Container for [PathEnvironment]
    Path(PathEnvironment),
    /// Container for [ManagedEnvironment]
    Managed(ManagedEnvironment),
    /// Container for [RemoteEnvironment]
    Remote(RemoteEnvironment),
}

/// A link to a built environment in the Nix store.
///
/// The path may not exist if the environment has never been built and linked.
///
/// As part of an environment, the existence of this path guarantees exactly two things:
/// - The environment was built at some point in the past.
/// - The environment can be activated.
///
/// The existence of this path explicitly _does not_ guarantee
/// that the current state of the environment is "buildable".
/// The environment may have been modified since it was last built
/// and therefore may no longer build.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, derive_more::Deref, derive_more::AsRef)]
#[as_ref(forward)]
pub struct RenderedEnvironmentLink(PathBuf);

/// A pair of links to the development and runtime variants of an environment.
/// Refer to the documentation of [RenderedEnvironmentLink] for what guarantees
/// the existence of these paths provides.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct RenderedEnvironmentLinks {
    pub development: RenderedEnvironmentLink,
    pub runtime: RenderedEnvironmentLink,
}

impl RenderedEnvironmentLinks {
    pub(crate) fn new_unchecked(development: PathBuf, runtime: PathBuf) -> Self {
        Self {
            development: RenderedEnvironmentLink(development),
            runtime: RenderedEnvironmentLink(runtime),
        }
    }

    pub fn new_in_base_dir_with_name_and_system(
        base_dir: &CanonicalPath,
        name: impl AsRef<str>,
        system: &System,
    ) -> Self {
        let development_name = format!("{system}.{name}.dev", name = name.as_ref());
        let development_path = base_dir.join(development_name);
        let runtime_name = format!("{system}.{name}.run", name = name.as_ref());
        let runtime_path = base_dir.join(runtime_name);
        Self::new_unchecked(development_path, runtime_path)
    }

    /// Returns the built environment path for an activation mode.
    pub fn for_mode(self, mode: &ActivateMode) -> RenderedEnvironmentLink {
        match mode {
            ActivateMode::Dev => self.development,
            ActivateMode::Run => self.runtime,
        }
    }
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
    #[serde(rename = "floxhub_url")]
    #[cfg_attr(test, proptest(value = "crate::flox::DEFAULT_FLOXHUB_URL.clone()"))]
    pub floxhub_base_url: Url,
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
            floxhub_base_url: floxhub.base_url().clone(),
            floxhub_git_url_override: floxhub.git_url_override().cloned(),
            version: Version::<1>,
        }
    }

    /// URL for the environment on FloxHub.
    pub fn floxhub_url(&self) -> Result<Url, ParseError> {
        self.floxhub_base_url
            .join(&format!("{}/{}", self.owner, self.name))
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
    /// If you want to operate on the [Environment] at the given path then use
    /// [open_path] on a project path instead.
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

/// An environment descriptor of an environment that can be (re)opened,
/// i.e. to install packages into it.
///
/// Unlike [ConcreteEnvironment], this type does not hold a concrete instance any environment,
/// but rather fully qualified metadata to create an instance from.
///
/// * for [PathEnvironment] and [ManagedEnvironment] that's the path to their `.flox` and `.flox/env.json`
/// * for [RemoteEnvironment] that's the [ManagedPointer] to the remote environment
///
/// Serialized as is into [FLOX_ACTIVE_ENVIRONMENTS_VAR] to be able to reopen environments.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "type")]
#[serde(rename_all = "kebab-case")]
pub enum UninitializedEnvironment {
    /// Container for "local" environments pointed to by [DotFlox]
    DotFlox(DotFlox),
    /// Container for [RemoteEnvironment]
    Remote(ManagedPointer),
}

impl UninitializedEnvironment {
    pub fn from_concrete_environment(env: &ConcreteEnvironment) -> Self {
        match env {
            ConcreteEnvironment::Path(path_env) => {
                let pointer = path_env.pointer.clone().into();
                Self::DotFlox(DotFlox {
                    path: path_env.path.to_path_buf(),
                    pointer,
                })
            },
            ConcreteEnvironment::Managed(managed_env) => {
                let pointer = managed_env.pointer().clone().into();
                Self::DotFlox(DotFlox {
                    path: managed_env.dot_flox_path().to_path_buf(),
                    pointer,
                })
            },
            ConcreteEnvironment::Remote(remote_env) => {
                let env_ref = remote_env.pointer().clone();
                Self::Remote(env_ref)
            },
        }
    }

    /// Open the contained environment and return a [ConcreteEnvironment]
    ///
    /// This function will fail if the contained environment is not available or invalid
    pub fn into_concrete_environment(
        self,
        flox: &Flox,
    ) -> Result<ConcreteEnvironment, EnvironmentError> {
        match self {
            UninitializedEnvironment::DotFlox(dot_flox) => {
                let dot_flox_path = CanonicalPath::new(dot_flox.path)
                    .map_err(|err| EnvironmentError::DotFloxNotFound(err.path))?;

                let env = match dot_flox.pointer {
                    EnvironmentPointer::Path(path_pointer) => {
                        debug!("detected concrete environment type: path");
                        ConcreteEnvironment::Path(PathEnvironment::open(
                            flox,
                            path_pointer,
                            dot_flox_path,
                        )?)
                    },
                    EnvironmentPointer::Managed(managed_pointer) => {
                        debug!("detected concrete environment type: managed");
                        let env = ManagedEnvironment::open(flox, managed_pointer, dot_flox_path)?;
                        ConcreteEnvironment::Managed(env)
                    },
                };
                Ok(env)
            },
            UninitializedEnvironment::Remote(pointer) => {
                let env = RemoteEnvironment::new(flox, pointer)?;
                Ok(ConcreteEnvironment::Remote(env))
            },
        }
    }

    pub fn pointer(&self) -> EnvironmentPointer {
        match self {
            UninitializedEnvironment::DotFlox(DotFlox { pointer, .. }) => pointer.clone(),
            UninitializedEnvironment::Remote(pointer) => {
                EnvironmentPointer::Managed(pointer.clone())
            },
        }
    }

    /// The name of the environment
    pub fn name(&self) -> &EnvironmentName {
        match self {
            UninitializedEnvironment::DotFlox(DotFlox { pointer, .. }) => pointer.name(),
            UninitializedEnvironment::Remote(pointer) => &pointer.name,
        }
    }

    /// Returns the path to the environment if it isn't remote
    #[allow(dead_code)]
    pub fn path(&self) -> Option<&Path> {
        match self {
            UninitializedEnvironment::DotFlox(DotFlox { path, .. }) => Some(path),
            UninitializedEnvironment::Remote(_) => None,
        }
    }

    /// If the environment is managed, returns its owner
    pub fn owner_if_managed(&self) -> Option<&EnvironmentOwner> {
        match self {
            UninitializedEnvironment::DotFlox(DotFlox {
                path: _,
                pointer: EnvironmentPointer::Managed(pointer),
            }) => Some(&pointer.owner),
            _ => None,
        }
    }

    /// Returns true if the environment is a path environment
    #[allow(dead_code)]
    pub fn is_path_env(&self) -> bool {
        matches!(
            self,
            UninitializedEnvironment::DotFlox(DotFlox {
                path: _,
                pointer: EnvironmentPointer::Path(_)
            })
        )
    }

    /// If the environment is remote, returns its owner
    pub fn owner_if_remote(&self) -> Option<&EnvironmentOwner> {
        match self {
            UninitializedEnvironment::DotFlox(_) => None,
            UninitializedEnvironment::Remote(pointer) => Some(&pointer.owner),
        }
    }

    /// The environment description when displayed in a prompt
    // TODO: we use this for activate errors in Bash since it doesn't have
    // quotes whereas we use message_description for activate errors in Rust.
    pub fn bare_description(&self) -> String {
        if let Some(owner) = self.owner_if_remote() {
            format!("{}/{} (remote)", owner, self.name())
        } else if let Some(owner) = self.owner_if_managed() {
            format!("{}/{}", owner, self.name())
        } else {
            format!("{}", self.name())
        }
    }
}

#[derive(Debug, Error)]
pub enum EnvironmentError {
    // todo: candidate for impl specific error
    // * only path and managed env are defined in .flox
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
    #[error(transparent)]
    ManifestError(#[from] ManifestError),

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
    #[error("could not write .gitattributes file")]
    WriteGitattributes(#[source] std::io::Error),
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
    LockedManifest(ResolveError),

    #[error(transparent)]
    Canonicalize(CanonicalizeError),

    #[error("could not create services socket directory")]
    CreateServicesSocketDirectory(#[source] std::io::Error),

    #[error("path for services socket is too long: {0}")]
    ServicesSocketPathTooLong(PathBuf),

    #[error("corrupt environment; environment does not have a lockfile")]
    MissingLockfile,

    /// An error flox edit can recover from
    #[error(transparent)]
    Recoverable(RecoverableMergeError),

    #[error("authentication error")]
    Auth(#[source] AuthError),
}

#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    #[error(transparent)]
    PkgNotFound(#[from] ManifestError),
    #[error("'{pkg}' is a package in the group '{group}' with multiple packages")]
    NonEmptyNamedGroup { pkg: String, group: String },
}

#[derive(Debug, thiserror::Error)]
pub enum UninstallError {
    #[error(transparent)]
    ManifestError(#[from] ManifestError),
    #[error(
        "Cannot remove included package '{0}'\n\
         Remove the package from environment '{1}' and then run 'flox include upgrade'"
    )]
    PackageOnlyIncluded(String, String),
}

/// Open an environment defined in `path` that has a `.flox` within.
pub fn open_path(
    flox: &Flox,
    path: impl AsRef<Path>,
) -> Result<ConcreteEnvironment, EnvironmentError> {
    DotFlox::open_in(path)
        .map(UninitializedEnvironment::DotFlox)?
        .into_concrete_environment(flox)
}

/// Copy a whole directory recursively ignoring the original permissions
///
/// We need this because:
/// 1. Sometimes we need to copy from the Nix store
/// 2. fs_extra::dir::copy doesn't handle symlinks.
///    See: https://github.com/webdesus/fs_extra/issues/61
pub(crate) fn copy_dir_recursive(
    from: impl AsRef<Path>,
    to: impl AsRef<Path>,
    keep_permissions: bool,
) -> Result<(), std::io::Error> {
    if !to.as_ref().exists() {
        std::fs::create_dir(&to).unwrap();
    }
    for entry in WalkDir::new(&from).into_iter().skip(1) {
        let entry = entry.unwrap();
        let new_path = to.as_ref().join(entry.path().strip_prefix(&from).unwrap());
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
    if let Ok(path) = std::env::var(FLOX_SERVICES_SOCKET_OVERRIDE_VAR) {
        return Ok(PathBuf::from(path));
    }

    let runtime_dir = &flox.runtime_dir;

    #[cfg(target_os = "macos")]
    let max_length = 104;
    #[cfg(target_os = "linux")]
    // 108 minus a null character
    let max_length = 107;

    let socket_path = runtime_dir.join(format!("flox.{}.sock", id));

    if socket_path.as_os_str().len() > max_length {
        return Err(EnvironmentError::ServicesSocketPathTooLong(socket_path));
    }

    std::fs::create_dir_all(runtime_dir)
        .map_err(EnvironmentError::CreateServicesSocketDirectory)?;

    Ok(socket_path)
}

/// Creates the `.gitignore` file in the `.flox` directory that prevents logs and cache
/// files from being tracked by git.
pub fn create_dot_flox_gitignore(dot_flox_path: impl AsRef<Path>) -> Result<(), EnvironmentError> {
    let dot_flox_path = dot_flox_path.as_ref();
    let gitignore_path = dot_flox_path.join(".gitignore");
    debug!(path = ?gitignore_path, "creating .flox/.gitignore");
    fs::write(gitignore_path, formatdoc! {"
        {GCROOTS_DIR_NAME}/
        {CACHE_DIR_NAME}/
        {LIB_DIR_NAME}/
        {LOG_DIR_NAME}/
        !{ENV_DIR_NAME}/
        "})
    .map_err(EnvironmentError::WriteGitignore)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use std::str::FromStr;
    use std::sync::LazyLock;
    use std::time::Duration;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::flox::DEFAULT_FLOXHUB_URL;
    use crate::flox::test_helpers::flox_instance;
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

    static MANAGED_ENV_POINTER: LazyLock<EnvironmentPointer> = LazyLock::new(|| {
        EnvironmentPointer::Managed(ManagedPointer {
            name: EnvironmentName::from_str("name").unwrap(),
            owner: EnvironmentOwner::from_str("owner").unwrap(),
            floxhub_base_url: DEFAULT_FLOXHUB_URL.clone(),
            floxhub_git_url_override: None,
            version: Version::<1> {},
        })
    });

    #[test]
    fn serializes_managed_environment_pointer() {
        let managed_pointer = EnvironmentPointer::Managed(ManagedPointer {
            name: EnvironmentName::from_str("name").unwrap(),
            owner: EnvironmentOwner::from_str("owner").unwrap(),
            floxhub_base_url: DEFAULT_FLOXHUB_URL.clone(),
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
    fn floxhub_url_for_pointer() {
        let mut managed_pointer = ManagedPointer {
            name: EnvironmentName::from_str("name").unwrap(),
            owner: EnvironmentOwner::from_str("owner").unwrap(),
            floxhub_base_url: Url::from_str("https://example.com/").unwrap(),
            floxhub_git_url_override: None,
            version: Version::<1> {},
        };
        assert_eq!(
            managed_pointer.floxhub_url().unwrap().as_str(),
            "https://example.com/owner/name",
            "should construct a URL for the environment",
        );

        managed_pointer.floxhub_base_url = Url::from_str("https://example.com/base/").unwrap();
        assert_eq!(
            managed_pointer.floxhub_url().unwrap().as_str(),
            "https://example.com/base/owner/name",
            "should respect additional paths in the base URL",
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
    fn services_socket_path_errors_if_too_long() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        flox.runtime_dir = flox.runtime_dir.join("X".repeat(100));
        let err = services_socket_path("1", &flox).unwrap_err();
        assert!(matches!(
            err,
            EnvironmentError::ServicesSocketPathTooLong(_)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn services_socket_path_errors_if_too_long() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        flox.runtime_dir = flox.runtime_dir.join("X".repeat(100));
        let err = services_socket_path("1", &flox).unwrap_err();
        assert!(matches!(
            err,
            EnvironmentError::ServicesSocketPathTooLong(_)
        ));
    }
}
