//! The directory structure for a path environment looks like this:
//!
//! ```ignore
//! .flox/
//!     ENVIRONMENT_POINTER_FILENAME
//!     ENVIRONMENT_DIR_NAME/
//!         MANIFEST_FILENAME
//!         LOCKFILE_FILENAME
//!     PATH_ENV_GCROOTS_DIR_NAME/
//!         $system.$name (out link)
//! ```
//!
//! `ENVIRONMENT_DIR_NAME` contains the environment definition
//! and is modified using [CoreEnvironment].

use std::ffi::OsStr;
use std::fs::{self};
use std::iter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use flox_core::activate::mode::ActivateMode;
use flox_core::data::environment_ref::EnvironmentName;
use flox_core::{blake3_hex, write_atomically};
use flox_manifest::interfaces::{AsWritableManifest, WriteManifest};
use flox_manifest::lockfile::{LOCKFILE_FILENAME, LockedInclude, Lockfile};
use flox_manifest::parsed::common::KnownSchemaVersion;
use flox_manifest::parsed::latest::AutoUpgrade;
use flox_manifest::raw::{CatalogPackage, DEFAULT_SYSTEMS_STR, PackageToInstall};
use flox_manifest::{MANIFEST_FILENAME, Manifest, Migrated, Validated, Writable};
use indoc::formatdoc;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::core_environment::{
    CoreEnvironment,
    CoreEnvironmentError,
    FollowMode,
    FollowedIncludes,
    NotAppliedIncludes,
    UnreadableInclude,
    UpgradeResult,
};
use super::fetcher::{IncludeFetcher, RemoteLockfiles};
use super::uninstall::UninstallSpec;
use super::{
    CACHE_DIR_NAME,
    DOT_FLOX,
    DotFlox,
    ENVIRONMENT_POINTER_FILENAME,
    EditResult,
    EnvJson,
    Environment,
    EnvironmentError,
    EnvironmentPointer,
    GCROOTS_DIR_NAME,
    InstallationAttempt,
    LOG_DIR_NAME,
    PathPointer,
    RenderedEnvironmentLinks,
    UninstallationAttempt,
    path_hash,
    services_socket_path,
};
use crate::data::{CanonicalPath, System};
use crate::flox::Flox;
use crate::models::env_registry::{deregister, ensure_registered};
use crate::models::environment::{ENV_DIR_NAME, create_dot_flox_gitignore};
use crate::providers::buildenv::{BuildEnvError, BuildEnvOutputs};
use crate::providers::lock_manifest::LockResult;
use crate::providers::manifest_init::ManifestInitializer;

/// The start of the names of the files in `.flox/cache` that keep a copy of the
/// lockfile with the latest changes to followed included environments, one per
/// system, since whether the copy builds depends on the system
const FOLLOWED_LOCKFILE_PREFIX: &str = "followed-includes.";

/// A copy of an environment's lockfile with the latest changes to the included
/// environments it follows.
///
/// It's kept in the gitignored `.flox/cache`, so following included
/// environments never changes the environment's own lockfile.
/// That only changes with explicit commands, such as 'flox include upgrade'.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct FollowedLockfile {
    /// Hash of the lockfile that this is a copy of
    base: String,
    lockfile: Lockfile,
    build: FollowedBuild,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FollowedBuild {
    Pending,
    Succeeded,
    /// Building failed with this error, and would fail the same way again,
    /// so the copy isn't used until the included environments change again
    Failed(String),
}

impl FollowedBuild {
    fn failed(&self) -> bool {
        matches!(self, FollowedBuild::Failed(_))
    }
}

/// The lockfile that following included environments results in
#[derive(Clone, Debug)]
struct Following {
    lock_result: LockResult,
    followed: FollowedIncludes,
    /// The copy of the lockfile in use, if any
    copy: Option<FollowedLockfile>,
}

/// The included environments locked in `lockfile`, if it has any
fn locked_includes(lockfile: &Lockfile) -> Option<&Vec<LockedInclude>> {
    lockfile.compose.as_ref().map(|compose| &compose.include)
}

/// Names of the included environments that `copy` has different versions of
/// than `lockfile`
fn changed_include_names(lockfile: &Lockfile, copy: &Lockfile) -> Vec<String> {
    let locked = locked_includes(lockfile)
        .map(Vec::as_slice)
        .unwrap_or_default();
    locked_includes(copy)
        .into_iter()
        .flatten()
        .filter(|include| !locked.contains(include))
        .map(|include| include.name.clone())
        .collect()
}

/// Whether building would fail the same way again, unlike a failure to
/// download a package, for example
fn fails_every_time(err: &CoreEnvironmentError) -> bool {
    matches!(
        err,
        CoreEnvironmentError::Manifest(_)
            | CoreEnvironmentError::BuildEnv(
                BuildEnvError::Build(_)
                    | BuildEnvError::Manifest(_)
                    | BuildEnvError::VarsCycle { .. }
                    | BuildEnvError::LockfileIncompatible { .. }
                    | BuildEnvError::LockfileMissingCurrentSystem { .. }
            )
    )
}

/// An error and its sources, on one line
fn error_chain(err: &dyn std::error::Error) -> String {
    iter::successors(Some(err), |err| err.source())
        .map(ToString::to_string)
        .join(": ")
}

/// Struct representing a local environment
///
/// This environment performs transactional edits by first copying the environment
/// to a temporary directory, making changes there, and attempting to build the
/// environment. If the build succeeds, the edit is considered a success and the
/// original environment contents are overwritten with the contents of the temporary
/// directory.
///
/// The transaction status is captured via the `state` field.
#[derive(Debug)]
pub struct PathEnvironment {
    /// Absolute path to the environment, typically `<...>/.flox`
    pub path: CanonicalPath,

    /// The associated [PathPointer] of this environment.
    ///
    /// Used to identify the environment.
    pub pointer: PathPointer,

    /// The rendered environment links for this environment.
    /// These may not yet exist if the environment has not been built.
    rendered_env_links: RenderedEnvironmentLinks,

    /// Included remote environments fetched by this instance, so that a
    /// command that uses the environment more than once fetches each of
    /// them once
    remote_lockfiles: RemoteLockfiles,
}

/// A profile script or list of packages to install when initializing an environment
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct InitCustomization {
    pub hook_on_activate: Option<String>,
    pub profile_common: Option<String>,
    pub profile_bash: Option<String>,
    pub profile_fish: Option<String>,
    pub profile_tcsh: Option<String>,
    pub profile_zsh: Option<String>,
    pub packages: Option<Vec<CatalogPackage>>,
    pub activate_mode: Option<ActivateMode>,
    pub description: Option<String>,
}

impl PartialEq for PathEnvironment {
    fn eq(&self, other: &Self) -> bool {
        *self.path == *other.path
    }
}

impl PathEnvironment {
    pub fn new(
        pointer: PathPointer,
        dot_flox_path: CanonicalPath,
        system: &System,
    ) -> Result<Self, EnvironmentError> {
        if &*dot_flox_path == Path::new("/") {
            return Err(EnvironmentError::InvalidPath(dot_flox_path.into_inner()));
        }

        let env_path = dot_flox_path.join(ENV_DIR_NAME);
        if !env_path.exists() {
            Err(EnvironmentError::EnvDirNotFound)?;
        }

        if !env_path.join(MANIFEST_FILENAME).exists() {
            Err(EnvironmentError::ManifestNotFound)?
        }

        let rendered_env_links = {
            let run_dir = dot_flox_path.join(GCROOTS_DIR_NAME);
            if !run_dir.exists() {
                std::fs::create_dir_all(&run_dir).map_err(EnvironmentError::CreateGcRootDir)?;
            }

            let base_dir = CanonicalPath::new(run_dir).expect("run dir is checked to exist");

            RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
                &base_dir,
                pointer.name.as_ref(),
                system,
            )
        };

        Ok(Self {
            // path must be absolute as it is used to set FLOX_ENV
            path: dot_flox_path,
            pointer,
            rendered_env_links,
            remote_lockfiles: RemoteLockfiles::default(),
        })
    }

    fn include_fetcher(&self) -> Result<IncludeFetcher, EnvironmentError> {
        Ok(
            IncludeFetcher::for_composer(self.parent_path()?, self.path.clone())
                .with_remote_lockfiles(self.remote_lockfiles.clone()),
        )
    }

    /// Get a view of the environment that can be used to perform operations
    /// on the environment without side effects.
    ///
    /// This method should only be used to create [CoreEnvironment]s for a [PathEnvironment].
    /// To modify the environment, use the [PathEnvironment] methods instead.
    pub(super) fn into_core_environment(self) -> Result<CoreEnvironment, EnvironmentError> {
        self.as_core_environment()
    }

    /// Get a view of an environment that is included by another environment,
    /// whose includes are fetched with `include_fetcher`.
    pub(super) fn into_core_environment_with_include_fetcher(
        self,
        include_fetcher: IncludeFetcher,
    ) -> CoreEnvironment {
        CoreEnvironment::new(self.path.join(ENV_DIR_NAME), include_fetcher)
    }

    fn as_core_environment(&self) -> Result<CoreEnvironment, EnvironmentError> {
        Ok(CoreEnvironment::new(
            self.path.join(ENV_DIR_NAME),
            self.include_fetcher()?,
        ))
    }

    fn as_core_environment_mut(&mut self) -> Result<CoreEnvironment, EnvironmentError> {
        self.as_core_environment()
    }

    /// Lock the environment, and use a copy of its lockfile with the latest
    /// changes to the included environments it follows if any changed.
    ///
    /// The `auto-upgrade` field of each include decides whether it's
    /// followed, see [AutoUpgrade].
    /// Only changes that the included environments have locked are used.
    /// Changes that can't be read or locked keep the versions in use before,
    /// and changes that don't build together with this environment fall back
    /// to the environment's lockfile, as [FollowedIncludes] reports.
    fn follow_includes(
        &mut self,
        flox: &Flox,
        mode: FollowMode,
    ) -> Result<Following, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let lock_result = env_view.ensure_locked(flox)?;
        let committed = match &lock_result {
            LockResult::Changed(lockfile) | LockResult::Unchanged(lockfile) => lockfile,
        };
        let locked_without_copy = |followed| Following {
            lock_result: lock_result.clone(),
            followed,
            copy: None,
        };
        let may_auto_upgrade = locked_includes(committed).is_some_and(|includes| {
            includes
                .iter()
                .any(|locked| locked.descriptor.auto_upgrade() != AutoUpgrade::Never)
        });
        if !may_auto_upgrade {
            self.remove_followed_lockfile(&flox.system);
            return Ok(locked_without_copy(FollowedIncludes::default()));
        }

        let base = blake3_hex(&serde_json::to_vec(committed).expect("lockfile is valid json"));
        let cached = self
            .read_followed_lockfile(&flox.system)
            .filter(|cached| cached.base == base);
        // Changes are checked against the versions in use
        let in_use = cached.as_ref().filter(|cached| !cached.build.failed());
        let check = env_view
            .check_auto_upgraded_includes(flox, in_use.map_or(committed, |copy| &copy.lockfile));
        let mut followed = FollowedIncludes {
            unreadable: check
                .unreadable
                .into_iter()
                .map(|(name, err)| UnreadableInclude {
                    name,
                    reason: Arc::new(err),
                })
                .collect(),
            ..Default::default()
        };

        let mut is_new = false;
        let copy = if check.changed.is_empty() {
            in_use.cloned()
        } else if let Some(cached) = cached
            .as_ref()
            .filter(|cached| locked_includes(&cached.lockfile) == Some(&check.includes))
        {
            Some(cached.clone())
        } else {
            let seed = in_use.map_or(committed, |copy| &copy.lockfile);
            match env_view.lock_with_latest_includes(flox, seed, check.changed.clone()) {
                Ok(lockfile) => {
                    is_new = true;
                    Some(FollowedLockfile {
                        base,
                        lockfile,
                        build: FollowedBuild::Pending,
                    })
                },
                Err(err) => {
                    followed.not_locked = Some(NotAppliedIncludes {
                        names: check.changed,
                        reason: Arc::new(err),
                    });
                    in_use.cloned()
                },
            }
        };

        let Some(mut copy) =
            copy.filter(|copy| locked_includes(&copy.lockfile) != locked_includes(committed))
        else {
            self.remove_followed_lockfile(&flox.system);
            return Ok(locked_without_copy(followed));
        };
        if is_new {
            self.write_followed_lockfile(&flox.system, &copy);
        }
        let unsaved = changed_include_names(committed, &copy.lockfile);

        if mode == FollowMode::LockAndBuild
            && copy.build == FollowedBuild::Pending
            && let Err(err) = self.build_followed_lockfile(flox, &mut copy)
        {
            followed.not_built = Some(NotAppliedIncludes {
                names: unsaved,
                reason: Arc::new(err),
            });
            return Ok(locked_without_copy(followed));
        }
        if let FollowedBuild::Failed(message) = &copy.build {
            followed.not_built = Some(NotAppliedIncludes {
                names: unsaved,
                reason: Arc::new(CoreEnvironmentError::FollowedBuildFailed(message.clone()).into()),
            });
            return Ok(locked_without_copy(followed));
        }

        followed.unsaved = unsaved;
        let lock_result = if is_new {
            LockResult::Changed(copy.lockfile.clone())
        } else {
            LockResult::Unchanged(copy.lockfile.clone())
        };
        Ok(Following {
            lock_result,
            followed,
            copy: Some(copy),
        })
    }

    /// Build a copy of the lockfile with the latest changes to included path
    /// environments into the rendered environment links,
    /// and record whether that worked.
    fn build_followed_lockfile(
        &mut self,
        flox: &Flox,
        copy: &mut FollowedLockfile,
    ) -> Result<BuildEnvOutputs, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = env_view.build_lockfile(flox, &copy.lockfile, Some(out_link_prefix));
        let build = match &result {
            Ok(_) => FollowedBuild::Succeeded,
            Err(err) if fails_every_time(err) => FollowedBuild::Failed(error_chain(err)),
            Err(_) => FollowedBuild::Pending,
        };
        if copy.build != build {
            copy.build = build;
            self.write_followed_lockfile(&flox.system, copy);
        }
        let outputs = result?;
        self.rendered_env_links.replace_legacy_links();
        Ok(outputs)
    }

    /// Names of the included environments that the copy of the lockfile in
    /// use has changes to, which the lockfile doesn't have yet.
    ///
    /// Unlike [Self::follow_includes], this doesn't lock anything,
    /// so it reports the copy from the last command that used it.
    pub fn unsaved_followed_includes(&self, flox: &Flox) -> Result<Vec<String>, EnvironmentError> {
        let Some(committed) = self.existing_lockfile(flox)? else {
            return Ok(Vec::new());
        };
        let base = blake3_hex(&serde_json::to_vec(&committed).expect("lockfile is valid json"));
        Ok(self
            .read_followed_lockfile(&flox.system)
            .filter(|copy| copy.base == base && !copy.build.failed())
            .map(|copy| changed_include_names(&committed, &copy.lockfile))
            .unwrap_or_default())
    }

    /// Build the latest changes to followed included environments into the
    /// rendered environment links again, after a command that changed the
    /// lockfile built the lockfile into them, so that activations keep using
    /// the changes.
    ///
    /// Failing only leaves the lockfile in the links until the next command
    /// that uses the environment.
    fn link_followed_changes(&mut self, flox: &Flox) {
        if let Err(err) = self.follow_includes(flox, FollowMode::LockAndBuild) {
            debug!(%err, "could not use the latest changes to included environments");
        }
    }

    /// Build the environment's lockfile, without the latest changes to
    /// followed included environments that it doesn't have yet.
    ///
    /// Publishing builds this, so that it builds what's committed.
    pub fn build_locked(&mut self, flox: &Flox) -> Result<BuildEnvOutputs, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        env_view.ensure_locked(flox)?;
        let store_paths = env_view.build(flox, Some(out_link_prefix))?;
        self.rendered_env_links.replace_legacy_links();
        Ok(store_paths)
    }

    fn followed_lockfile_path(&self, system: &System) -> PathBuf {
        self.path
            .join(CACHE_DIR_NAME)
            .join(format!("{FOLLOWED_LOCKFILE_PREFIX}{system}.json"))
    }

    fn read_followed_lockfile(&self, system: &System) -> Option<FollowedLockfile> {
        let contents = fs::read_to_string(self.followed_lockfile_path(system)).ok()?;
        serde_json::from_str(&contents)
            .inspect_err(|err| debug!(%err, "ignoring unreadable copy of the lockfile"))
            .ok()
    }

    /// Failing to keep the copy only means that it's made again next time.
    fn write_followed_lockfile(&self, system: &System, copy: &FollowedLockfile) {
        let contents = serde_json::to_string(copy).expect("lockfile is valid json");
        // Creates the cache directory
        if let Err(err) = self.cache_path() {
            debug!(%err, "could not keep copy of the lockfile");
            return;
        }
        if let Err(err) = write_atomically(self.followed_lockfile_path(system), contents) {
            debug!(%err, "could not keep copy of the lockfile");
        }
    }

    fn remove_followed_lockfile(&self, system: &System) {
        let path = self.followed_lockfile_path(system);
        if path.exists()
            && let Err(err) = fs::remove_file(&path)
        {
            debug!(%err, "could not remove copy of the lockfile");
        }
    }

    /// Remove the copies of the lockfile for every system
    fn remove_followed_lockfiles(&self) {
        let Ok(entries) = fs::read_dir(self.path.join(CACHE_DIR_NAME)) else {
            return;
        };
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(FOLLOWED_LOCKFILE_PREFIX)
                && let Err(err) = fs::remove_file(entry.path())
            {
                debug!(%err, "could not remove copy of the lockfile");
            }
        }
    }

    pub fn rename(&mut self, new_name: EnvironmentName) -> Result<(), EnvironmentError> {
        self.pointer.name = new_name;

        // Rewrite from the full [EnvJson] rather than the bare pointer so
        // the sibling `env_id` survives the rename.
        let previous = EnvJson::read_from(&self.path).ok();
        if previous.is_none() {
            debug!("could not read or parse env.json before rename, dropping any env_id");
        }
        let env_id = previous.and_then(|env_json| env_json.env_id);
        let env_json = EnvJson {
            pointer: EnvironmentPointer::Path(self.pointer.clone()),
            env_id,
        };
        env_json.write_to(&self.path)
    }

    /// Returns a unique identifier for the location of the environment.
    fn path_hash(&self) -> String {
        path_hash(&self.path)
    }
}

impl Environment for PathEnvironment {
    /// This will lock the environment if it is not already locked,
    /// and use the latest changes to the included environments it follows.
    fn lockfile(&mut self, flox: &Flox) -> Result<LockResult, EnvironmentError> {
        let Following {
            lock_result,
            followed,
            ..
        } = self.follow_includes(flox, FollowMode::Lock)?;
        debug!(?followed, "followed included environments");
        Ok(lock_result)
    }

    fn lockfile_following_includes(
        &mut self,
        flox: &Flox,
        mode: FollowMode,
    ) -> Result<(LockResult, FollowedIncludes), EnvironmentError> {
        let Following {
            lock_result,
            followed,
            ..
        } = self.follow_includes(flox, mode)?;
        Ok((lock_result, followed))
    }

    /// Unlike following, this doesn't lock the environment, or use or keep a
    /// copy of the lockfile, and it always builds the latest changes,
    /// without replacing the rendered environment links,
    /// so it reports the same wherever it runs.
    fn check_followed_includes(
        &mut self,
        flox: &Flox,
    ) -> Result<Option<FollowedIncludes>, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let Some(committed) = env_view.lockfile_if_up_to_date()? else {
            return Ok(None);
        };
        let check = env_view.check_auto_upgraded_includes(flox, &committed);
        let mut followed = FollowedIncludes {
            unreadable: check
                .unreadable
                .into_iter()
                .map(|(name, err)| UnreadableInclude {
                    name,
                    reason: Arc::new(err),
                })
                .collect(),
            ..Default::default()
        };
        if check.changed.is_empty() {
            return Ok(Some(followed));
        }

        let lockfile =
            match env_view.lock_with_latest_includes(flox, &committed, check.changed.clone()) {
                Ok(lockfile) => lockfile,
                Err(err) => {
                    followed.not_locked = Some(NotAppliedIncludes {
                        names: check.changed,
                        reason: Arc::new(err),
                    });
                    return Ok(Some(followed));
                },
            };
        let unsaved = changed_include_names(&committed, &lockfile);
        if unsaved.is_empty() {
            return Ok(Some(followed));
        }
        match env_view.build_lockfile(flox, &lockfile, None) {
            Ok(_) => followed.unsaved = unsaved,
            Err(err) => {
                followed.not_built = Some(NotAppliedIncludes {
                    names: unsaved,
                    reason: Arc::new(err.into()),
                })
            },
        }
        Ok(Some(followed))
    }

    /// Returns the lockfile if it already exists.
    fn existing_lockfile(&self, _flox: &Flox) -> Result<Option<Lockfile>, EnvironmentError> {
        self.as_core_environment()?
            .existing_lockfile()
            .map_err(EnvironmentError::Core)
    }

    fn manifest_without_migrating(
        &self,
        _flox: &Flox,
    ) -> Result<Manifest<Validated>, EnvironmentError> {
        let manifest = self.as_core_environment()?.manifest_without_migrating()?;
        Ok(manifest)
    }

    fn manifest(&mut self, flox: &Flox) -> Result<Manifest<Migrated>, EnvironmentError> {
        let manifest = self.as_core_environment()?.manifest(flox)?;
        Ok(manifest)
    }

    /// Install packages to the environment atomically
    ///
    /// Returns the new manifest content if the environment was modified. Also
    /// returns a map of the packages that were already installed. The installation
    /// will proceed if at least one of the requested packages were added to the
    /// manifest.
    ///
    /// Todo: remove async
    fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = env_view.install(packages, flox, Some(out_link_prefix))?;
        if result.built_environments.is_some() {
            self.rendered_env_links.replace_legacy_links();
            self.link_followed_changes(flox);
        }
        Ok(result)
    }

    /// Uninstall packages from the environment atomically
    ///
    /// Returns true if the environment was modified and false otherwise.
    /// TODO: this should return a list of packages that were actually
    /// uninstalled rather than a bool.
    fn uninstall(
        &mut self,
        specs: Vec<UninstallSpec>,
        flox: &Flox,
    ) -> Result<UninstallationAttempt, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = env_view.uninstall(specs, flox, Some(out_link_prefix))?;
        if result.built_environment_store_paths.is_some() {
            self.rendered_env_links.replace_legacy_links();
            self.link_followed_changes(flox);
        }
        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = env_view.edit(flox, contents, Some(out_link_prefix))?;
        if matches!(&result, EditResult::Changed { .. }) {
            self.rendered_env_links.replace_legacy_links();
            self.link_followed_changes(flox);
        }
        Ok(result)
    }

    /// Upgrade packages in this environment and return the result, but do not
    fn dry_upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let result = env_view.upgrade(flox, groups_or_iids, false, None)?; // dry-run: no out-link
        Ok(result)
    }

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError> {
        tracing::debug!(to_upgrade = groups_or_iids.join(","), "upgrading");
        let mut env_view = self.as_core_environment_mut()?;
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = env_view.upgrade(flox, groups_or_iids, true, Some(out_link_prefix))?;
        if result.store_path.is_some() {
            self.rendered_env_links.replace_legacy_links();
            self.link_followed_changes(flox);
        }
        Ok(result)
    }

    /// Upgrade environment with latest changes to included environments.
    fn include_upgrade(
        &mut self,
        flox: &Flox,
        to_upgrade: Vec<String>,
    ) -> Result<UpgradeResult, EnvironmentError> {
        tracing::debug!(
            includes = to_upgrade.iter().join(","),
            "upgrading included environments"
        );
        let mut env_view = self.as_core_environment_mut()?;
        let committed: Lockfile = env_view.ensure_locked(flox)?.into();
        // Save the packages of the copy in use rather than locking them again,
        // so that the lockfile gets what was used,
        // unless that would save changes to includes that weren't named.
        let base = blake3_hex(&serde_json::to_vec(&committed).expect("lockfile is valid json"));
        let copy = self
            .read_followed_lockfile(&flox.system)
            .filter(|copy| copy.base == base && !copy.build.failed())
            .filter(|copy| {
                to_upgrade.is_empty()
                    || changed_include_names(&committed, &copy.lockfile)
                        .iter()
                        .all(|name| to_upgrade.contains(name))
            });
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = env_view.include_upgrade(
            flox,
            to_upgrade,
            copy.as_ref().map(|copy| &copy.lockfile),
            Some(out_link_prefix),
        )?;
        if result.store_path.is_some() {
            self.rendered_env_links.replace_legacy_links();
        }
        // Copies are of the lockfile before the upgrade, so any changes that
        // are still unsaved are copied again the next time they're used.
        self.remove_followed_lockfiles();
        if result.store_path.is_some() {
            self.link_followed_changes(flox);
        }
        Ok(result)
    }

    /// Returns the environment name
    fn name(&self) -> EnvironmentName {
        self.pointer.name.clone()
    }

    /// Delete the Environment
    fn delete(self, flox: &Flox) -> Result<(), EnvironmentError> {
        let dot_flox = &self.path;
        if Some(OsStr::new(".flox")) == dot_flox.file_name() {
            std::fs::remove_dir_all(dot_flox).map_err(EnvironmentError::DeleteEnvironment)?;
        } else {
            return Err(EnvironmentError::DotFloxNotFound(self.path.to_path_buf()));
        }
        deregister(flox, &self.path, &EnvironmentPointer::Path(self.pointer))?;
        Ok(())
    }

    /// This will lock the environment if it is not already locked.
    fn rendered_env_links(
        &mut self,
        flox: &Flox,
    ) -> Result<RenderedEnvironmentLinks, EnvironmentError> {
        let out_paths = self.rendered_env_links.clone();

        let lockfile = self
            .follow_includes(flox, FollowMode::Lock)?
            .lock_result
            .into();
        if self.needs_rebuild(&lockfile) {
            self.build(flox)?;
        }

        Ok(out_paths)
    }

    /// Build the environment
    /// This will lock the environment if it is not already locked,
    /// and build it with the latest changes to the included environments it
    /// follows if it builds with them, see [Self::follow_includes].
    fn build(&mut self, flox: &Flox) -> Result<BuildEnvOutputs, EnvironmentError> {
        if let Some(mut copy) = self.follow_includes(flox, FollowMode::Lock)?.copy {
            match self.build_followed_lockfile(flox, &mut copy) {
                Ok(store_paths) => return Ok(store_paths),
                Err(err) => debug!(
                    %err,
                    "building with the latest changes to included environments failed, building the lockfile instead"
                ),
            }
        }
        self.build_locked(flox)
    }

    /// Returns .flox/cache
    fn cache_path(&self) -> Result<CanonicalPath, EnvironmentError> {
        let cache_dir = self.path.join(CACHE_DIR_NAME);
        if !cache_dir.exists() {
            std::fs::create_dir_all(&cache_dir).map_err(EnvironmentError::CreateCacheDir)?;
        }
        CanonicalPath::new(cache_dir).map_err(EnvironmentError::Canonicalize)
    }

    /// Returns .flox/log
    fn log_path(&self) -> Result<CanonicalPath, EnvironmentError> {
        let log_dir = self.path.join(LOG_DIR_NAME);
        if !log_dir.exists() {
            std::fs::create_dir_all(&log_dir).map_err(EnvironmentError::CreateLogDir)?;
        }
        CanonicalPath::new(log_dir).map_err(EnvironmentError::Canonicalize)
    }

    /// Returns parent path of .flox
    fn project_path(&self) -> Result<PathBuf, EnvironmentError> {
        self.parent_path()
    }

    /// Path to the environment's parent directory
    fn parent_path(&self) -> Result<PathBuf, EnvironmentError> {
        let mut path = self.path.to_path_buf();
        if path.pop() {
            Ok(path)
        } else {
            Err(EnvironmentError::InvalidPath(path))
        }
    }

    /// Path to the environment's .flox directory
    fn dot_flox_path(&self) -> CanonicalPath {
        self.path.clone()
    }

    /// Path to the environment definition file
    fn manifest_path(&self, _flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        Ok(self.path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME))
    }

    /// Path to the lockfile. The path may not exist.
    fn lockfile_path(&self, _flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        Ok(self.path.join(ENV_DIR_NAME).join(LOCKFILE_FILENAME))
    }

    /// Return the path where the process compose socket for an environment
    /// should be created
    fn services_socket_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        services_socket_path(&self.path_hash(), flox)
    }
}

/// Constructors of PathEnvironments
impl PathEnvironment {
    /// Open an environment at a given path
    ///
    /// Ensure that the path exists and contains files that "look" like an environment
    pub fn open(
        flox: &Flox,
        pointer: PathPointer,
        dot_flox_path: CanonicalPath,
    ) -> Result<Self, EnvironmentError> {
        ensure_registered(
            flox,
            &dot_flox_path,
            &EnvironmentPointer::Path(pointer.clone()),
        )?;

        PathEnvironment::new(pointer, dot_flox_path, &flox.system)
    }

    /// Create a new env in a `.flox` directory within a specific path
    /// or open it if it exists. Will create a very minimal manifest.
    ///
    /// The method creates or opens a `.flox` directory _contained_ within `path`!
    pub fn init_bare(
        pointer: PathPointer,
        dot_flox_parent_path: impl AsRef<Path>,
        flox: &Flox,
    ) -> Result<Self, EnvironmentError> {
        // Ensure that the .flox directory does not already exist
        match DotFlox::open_in(dot_flox_parent_path.as_ref()) {
            // continue if the .flox directory does not exist, as it's being created by this method
            Err(EnvironmentError::DotFloxNotFound(_)) => {},
            // propagate any other error signaling a faulty .flox directory
            Err(e) => Err(e)?,
            // .flox directory exists, so we can't create a new environment here
            Ok(_) => Err(EnvironmentError::EnvironmentExists(
                dot_flox_parent_path.as_ref().to_path_buf(),
            ))?,
        }

        // The most minimal manifest we can generate.
        let manifest = Manifest::parse_toml_typed(format!(
            "schema-version = \"{}\"\n",
            KnownSchemaVersion::latest()
        ))?;

        let environment = Self::write_new_unchecked(
            flox,
            pointer,
            dot_flox_parent_path,
            &manifest.as_writable(),
        )?;

        // Lock but don't build
        let mut env_view = CoreEnvironment::new(
            environment.path.join(ENV_DIR_NAME),
            environment.include_fetcher()?,
        );
        env_view.lock(flox)?;

        Ok(environment)
    }

    /// Create a new env in a `.flox` directory within a specific path or open it if it exists.
    ///
    /// The method creates or opens a `.flox` directory _contained_ within `path`!
    pub fn init(
        pointer: PathPointer,
        dot_flox_parent_path: impl AsRef<Path>,
        customization: &InitCustomization,
        flox: &Flox,
    ) -> Result<Self, EnvironmentError> {
        // Ensure that the .flox directory does not already exist
        match DotFlox::open_in(dot_flox_parent_path.as_ref()) {
            // continue if the .flox directory does not exist, as it's being created by this method
            Err(EnvironmentError::DotFloxNotFound(_)) => {},
            // propagate any other error signaling a faulty .flox directory
            Err(e) => Err(e)?,
            // .flox directory exists, so we can't create a new environment here
            Ok(_) => Err(EnvironmentError::EnvironmentExists(
                dot_flox_parent_path.as_ref().to_path_buf(),
            ))?,
        }

        // Create manifest
        let manifest = {
            tracing::debug!("creating raw catalog manifest");
            ManifestInitializer::new_documented(
                flox.features,
                &DEFAULT_SYSTEMS_STR.iter().collect::<Vec<_>>(),
                customization,
            )?
        };

        let environment = Self::write_new_unchecked(
            flox,
            pointer,
            dot_flox_parent_path,
            &manifest.as_writable(),
        )?;

        // Always lock
        let mut env_view = CoreEnvironment::new(
            environment.path.join(ENV_DIR_NAME),
            environment.include_fetcher()?,
        );
        env_view.lock(flox)?;

        // Build only when packages are present; nix writes the activation
        // symlinks via --out-link, so no separate link step is needed.
        if matches!(customization.packages.as_deref(), Some([_, ..])) {
            let out_link_prefix = environment.rendered_env_links.out_link_prefix();
            env_view.build(flox, Some(out_link_prefix))?;
        }

        Ok(environment)
    }

    /// Write files for a [PathEnvironment] to `dot_flox_parent_path` unchecked.
    ///
    /// * write the .flox directory
    /// * write the environment pointer to `.flox/env.json`
    /// * write the manifest to `.flox/env/manifest.toml`
    ///
    /// Note: The directory and the written environment are **not verified**.
    ///       This function may override any existing env,
    ///       or write nonsense content to the manifest.
    ///       [PathEnvironment::init] implements the relevant checks
    ///       to make this safe in practice.
    ///
    /// This functionality is shared between [PathEnvironment::init] and tests.
    fn write_new_unchecked(
        flox: &Flox,
        pointer: PathPointer,
        dot_flox_parent_path: impl AsRef<Path>,
        manifest: &Manifest<Writable>,
    ) -> Result<Self, EnvironmentError> {
        let dot_flox_path = dot_flox_parent_path.as_ref().join(DOT_FLOX);
        let env_dir = dot_flox_path.join(ENV_DIR_NAME);
        let manifest_path = env_dir.join(MANIFEST_FILENAME);
        debug!("creating env dir: {}", env_dir.display());
        std::fs::create_dir_all(&env_dir).map_err(EnvironmentError::InitEnv)?;

        // Write the `env.json` file
        let mut pointer_content =
            serde_json::to_string_pretty(&pointer).map_err(EnvironmentError::SerializeEnvJson)?;
        pointer_content.push('\n');
        if let Err(e) = fs::write(
            dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME),
            pointer_content,
        ) {
            fs::remove_dir_all(&env_dir).map_err(EnvironmentError::InitEnv)?;
            Err(EnvironmentError::WriteEnvJson(Box::new(e)))?;
        }

        // Write `manifest.toml`
        let write_res = manifest.write_to_file(&manifest_path);
        if let Err(e) = write_res {
            debug!("writing manifest did not complete successfully");
            fs::remove_dir_all(&env_dir).map_err(EnvironmentError::InitEnv)?;
            return Err(EnvironmentError::ManifestError(e));
        }

        // Write stateful directories to .flox/.gitignore
        create_dot_flox_gitignore(&dot_flox_path)?;

        // Write (configure) Git attributes to ./flox/.gitattributes
        fs::write(dot_flox_path.join(".gitattributes"), formatdoc! {"
            {ENV_DIR_NAME}/{LOCKFILE_FILENAME} linguist-generated=true linguist-language=JSON
            "})
        .map_err(EnvironmentError::WriteGitattributes)?;

        let dot_flox_path = CanonicalPath::new(dot_flox_path).expect("the directory just created");

        Self::open(flox, pointer, dot_flox_path)
    }

    /// Determine if the environment needs to be rebuilt,
    /// based on the lockfile in use and the rendered environment link.
    ///
    /// If no lockfile exists in the rendered environment,
    /// or it differs from the lockfile in use,
    /// the environment will be rebuilt.
    fn needs_rebuild(&self, lockfile: &Lockfile) -> bool {
        let rendered_env_lockfile_path = self.rendered_env_links.dev.join(LOCKFILE_FILENAME);

        let Ok(rendered_env_lockfile_path) = CanonicalPath::new(rendered_env_lockfile_path) else {
            return true;
        };

        let Ok(rendered_lockfile) = Lockfile::read_from_file(&rendered_env_lockfile_path) else {
            return true;
        };

        *lockfile != rendered_lockfile
    }

    /// The environment is locked,
    /// and the manifest in the lockfile matches that in the manifest.
    /// Note that the manifest could have whitespace or comment differences from
    /// the lockfile.
    pub fn lockfile_up_to_date(&self) -> Result<bool, EnvironmentError> {
        let env_view = self.as_core_environment()?;
        Ok(env_view.lockfile_if_up_to_date()?.is_some())
    }
}

pub mod test_helpers {
    use tempfile::tempdir_in;

    use super::*;

    pub fn new_path_environment_in(
        flox: &Flox,
        contents: &str,
        path: impl AsRef<Path>,
    ) -> PathEnvironment {
        let pointer = PathPointer::new(
            path.as_ref()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .parse()
                .unwrap(),
        );
        let manifest = Manifest::parse_toml_typed(contents).unwrap();
        PathEnvironment::write_new_unchecked(flox, pointer, path, &manifest.as_writable()).unwrap()
    }

    pub fn new_named_path_environment_in(
        flox: &Flox,
        contents: &str,
        path: impl AsRef<Path>,
        name: &str,
    ) -> PathEnvironment {
        let pointer = PathPointer::new(name.parse().unwrap());
        let manifest = Manifest::parse_toml_typed(contents).unwrap();
        PathEnvironment::write_new_unchecked(flox, pointer, path, &manifest.as_writable()).unwrap()
    }

    pub fn new_path_environment(flox: &Flox, contents: &str) -> PathEnvironment {
        new_path_environment_in(flox, contents, tempdir_in(&flox.temp_dir).unwrap().keep())
    }

    pub fn new_named_path_environment(flox: &Flox, contents: &str, name: &str) -> PathEnvironment {
        new_named_path_environment_in(
            flox,
            contents,
            tempdir_in(&flox.temp_dir).unwrap().keep(),
            name,
        )
    }

    pub fn new_path_environment_from_env_files(
        flox: &Flox,
        env_files_dir: impl AsRef<Path>,
    ) -> PathEnvironment {
        let dot_flox_parent_path = tempdir_in(&flox.temp_dir).unwrap().keep();
        new_path_environment_from_env_files_in(flox, env_files_dir, dot_flox_parent_path, None)
    }

    pub fn new_named_path_environment_from_env_files(
        flox: &Flox,
        env_files_dir: impl AsRef<Path>,
        name: &str,
    ) -> PathEnvironment {
        let dot_flox_parent_path = tempdir_in(&flox.temp_dir).unwrap().keep();
        new_path_environment_from_env_files_in(
            flox,
            env_files_dir,
            dot_flox_parent_path,
            Some(name),
        )
    }

    pub fn new_path_environment_from_env_files_in(
        flox: &Flox,
        env_files_dir: impl AsRef<Path>,
        dot_flox_parent_path: impl AsRef<Path>,
        name: Option<&str>,
    ) -> PathEnvironment {
        let env_files_dir = env_files_dir.as_ref();
        let manifest = Manifest::read_typed(env_files_dir.join(MANIFEST_FILENAME)).unwrap();
        let lockfile_contents = fs::read_to_string(env_files_dir.join(LOCKFILE_FILENAME)).unwrap();
        let pointer = PathPointer::new(
            name.unwrap_or_else(|| {
                dot_flox_parent_path
                    .as_ref()
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            })
            .parse()
            .unwrap(),
        );
        PathEnvironment::write_new_unchecked(
            flox,
            pointer.clone(),
            &dot_flox_parent_path,
            &manifest.as_writable(),
        )
        .unwrap();
        let dot_flox_path =
            CanonicalPath::new(dot_flox_parent_path.as_ref().join(DOT_FLOX)).unwrap();
        let env_dir = dot_flox_path.join(ENV_DIR_NAME);
        let lockfile_path = env_dir.join(LOCKFILE_FILENAME);
        fs::write(lockfile_path, lockfile_contents).unwrap();
        new_path_environment(flox, &manifest.as_writable().to_string());
        PathEnvironment::open(flox, pointer, dot_flox_path).unwrap()
    }
}

#[cfg(test)]
pub mod tests {

    use std::collections::BTreeMap;
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    use flox_manifest::interfaces::AsLatestSchema;
    use flox_manifest::parsed::Inner;
    use flox_manifest::parsed::common::KnownSchemaVersion;
    use flox_manifest::parsed::v1::test_helpers::manifest_without_install_or_include;
    use flox_manifest::test_helpers::{with_latest_schema, with_schema};
    use flox_test_utils::proptest::{alphanum_string, lowercase_alphanum_string};
    use indoc::indoc;
    use itertools::izip;
    use proptest::collection::{hash_set as prop_hash_set, vec as prop_vec};
    use proptest::prelude::*;
    use tempfile::TempDir;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::env_registry::{env_registry_path, read_environment_registry};
    use crate::models::environment::path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_from_env_files,
        new_path_environment_in,
    };
    use crate::providers::lock_manifest::RecoverableMergeError;
    use crate::utils::serialize_json_with_newline;

    /// Returns (flox, tempdir, Vec<(dir relative to tempdir, PathEnvironment)>)
    /// This is a list of relative paths to environments that can be included in
    /// another environment.
    /// The environment names and directories are unique.
    pub fn generate_path_environments_without_install_or_include(
        max_size: usize,
    ) -> impl Strategy<Value = (Flox, TempDir, Vec<(PathBuf, PathEnvironment)>)> {
        (1..=max_size).prop_flat_map(|size| {
            (
                prop_vec(manifest_without_install_or_include(), size..=size),
                prop_hash_set(alphanum_string(2), size..=size),
                // macOS is case-insensitive,
                // so only use lowercase directories so there isn't a collision
                // between e.g. dir and DIR
                prop_hash_set(lowercase_alphanum_string(2), size..=size),
            )
                .prop_map(|(manifests, names, dirs)| {
                    let (flox, tempdir) = flox_instance();

                    let mut environments = vec![];
                    for (manifest, name, dir) in izip!(&manifests, &names, &dirs) {
                        let relative_path = PathBuf::from(dir);
                        let absolute_path = tempdir.path().join(&relative_path);
                        fs::create_dir(&absolute_path).unwrap();
                        let mut environment = test_helpers::new_named_path_environment_in(
                            &flox,
                            &toml_edit::ser::to_string_pretty(&manifest).unwrap(),
                            absolute_path,
                            name,
                        );
                        environment.lockfile(&flox).unwrap();
                        environments.push((relative_path, environment));
                    }
                    (flox, tempdir, environments)
                })
        })
    }

    #[test]
    fn rename_preserves_env_id() {
        let (flox, _temp_dir) = flox_instance();
        let mut environment = new_path_environment(&flox, &with_latest_schema(""));

        let env_id = uuid::Uuid::new_v4();
        let env_json_path = environment.path.join(ENVIRONMENT_POINTER_FILENAME);
        let stamped = EnvJson {
            pointer: EnvironmentPointer::Path(environment.pointer.clone()),
            env_id: Some(env_id),
        };
        fs::write(&env_json_path, stamped.to_pretty_string().unwrap()).unwrap();

        environment.rename("renamed".parse().unwrap()).unwrap();

        assert_eq!(EnvJson::read_from(&environment.path).unwrap(), EnvJson {
            pointer: EnvironmentPointer::Path(PathPointer::new("renamed".parse().unwrap())),
            env_id: Some(env_id),
        });
    }

    #[test]
    fn create_env() {
        let (flox, temp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&temp_dir).unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let actual = PathEnvironment::init(
            pointer,
            environment_temp_dir.path(),
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();

        let expected = PathEnvironment::new(
            PathPointer::new("test".parse().unwrap()),
            CanonicalPath::new(environment_temp_dir.path().join(DOT_FLOX)).unwrap(),
            &flox.system,
        )
        .unwrap();

        assert_eq!(actual, expected);

        assert!(
            actual.manifest_path(&flox).unwrap().exists(),
            "manifest exists"
        );
        assert!(
            actual.lockfile_path(&flox).unwrap().exists(),
            "lockfile exists"
        );
        assert!(actual.path.is_absolute());
    }

    /// Write a manifest file with invalid toml to ensure we can catch
    #[test]
    fn cache_activation_path() {
        let (flox, temp_dir) = flox_instance();

        let environment_temp_dir = tempfile::tempdir_in(&temp_dir).unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let env = PathEnvironment::init(
            pointer,
            environment_temp_dir.path(),
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();

        let lockfile = env.existing_lockfile(&flox).unwrap().unwrap();
        assert!(env.needs_rebuild(&lockfile));

        // build the environment -> out link is created -> no rebuild necessary
        let mut env_view =
            CoreEnvironment::new(env.path.join(ENV_DIR_NAME), env.include_fetcher().unwrap());
        let out_link_prefix = env.rendered_env_links.out_link_prefix();
        env_view.build(&flox, Some(out_link_prefix)).unwrap();

        assert!(!env.needs_rebuild(&lockfile));

        // modify the lockfile  -> rebuild necessary
        let mut lockfile = env.existing_lockfile(&flox).unwrap().unwrap();
        let mut manifest = lockfile.migrated_manifest().unwrap();
        manifest.as_latest_schema_mut().options.activate.mode = Some(ActivateMode::Dev);
        lockfile.manifest = manifest.into();
        let lockfile_contents = serialize_json_with_newline(&lockfile).unwrap();
        fs::write(env.lockfile_path(&flox).unwrap(), lockfile_contents).unwrap();
        assert!(env.needs_rebuild(&lockfile));
    }

    #[test]
    fn registers_on_init() {
        let (flox, tmp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&tmp_dir).unwrap();
        let ptr = PathPointer::new("test".parse().unwrap());
        let _env = PathEnvironment::init(
            ptr,
            environment_temp_dir.path(),
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();
        let reg_path = env_registry_path(&flox);
        assert!(reg_path.exists());
        let reg = read_environment_registry(&reg_path).unwrap().unwrap();
        assert!(matches!(
            reg.entries[0].envs[0].pointer,
            EnvironmentPointer::Path(_)
        ));
    }

    #[test]
    fn registers_on_open() {
        let (flox, tmp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&tmp_dir).unwrap();
        // Create an environment so that the .flox directory is populated and we can open it later
        let ptr = PathPointer::new("test".parse().unwrap());
        let env = PathEnvironment::init(
            ptr.clone(),
            environment_temp_dir.path(),
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();
        let reg_path = env_registry_path(&flox);
        assert!(reg_path.exists());
        // Delete the registry so we can confirm that opening the environment creates it
        std::fs::remove_file(&reg_path).unwrap();
        let _env = PathEnvironment::open(&flox, ptr, env.path).unwrap();
        let reg = read_environment_registry(&reg_path).unwrap().unwrap();
        assert!(matches!(
            reg.entries[0].envs[0].pointer,
            EnvironmentPointer::Path(_)
        ));
    }

    #[test]
    fn deregisters_on_delete() {
        let (flox, tmp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&tmp_dir).unwrap();
        // Create an environment so that the .flox directory is populated and we can open it later
        let ptr = PathPointer::new("test".parse().unwrap());
        let env = PathEnvironment::init(
            ptr.clone(),
            environment_temp_dir.path(),
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();
        let reg_path = env_registry_path(&flox);
        assert!(reg_path.exists());
        env.delete(&flox).unwrap();
        assert!(reg_path.exists());
        let reg = read_environment_registry(&reg_path).unwrap().unwrap();
        assert!(reg.entries.is_empty());
    }

    /// If an environment doesn't have any included environments, calling include_upgrade()
    /// should error
    #[test]
    fn include_upgrade_errors_without_includes() {
        let (flox, _tempdir) = flox_instance();

        // Create environment
        let manifest_contents = indoc! {r#"
        version = 1
        "#};
        let mut composer = new_path_environment(&flox, manifest_contents);
        composer.lockfile(&flox).unwrap();

        // Try to upgrade
        let err = composer.include_upgrade(&flox, vec![]).unwrap_err();

        let EnvironmentError::Recoverable(RecoverableMergeError::Catchall(message)) = err else {
            panic!("expected Catchall error, got: {:?}", err)
        };

        assert_eq!(message, "environment has no included environments",);
    }

    /// include_upgrade()errors when specified included environment doesn't exist
    #[test]
    fn include_upgrade_errors_when_included_environment_does_not_exist() {
        let (flox, tempdir) = flox_instance();

        // Create dep
        let dep_path = tempdir.path().join("dep");
        let dep_manifest_contents = indoc! {r#"
            version = 1
            [vars]
            foo = "v1"
            "#};
        fs::create_dir(&dep_path).unwrap();
        let mut dep = new_path_environment_in(&flox, dep_manifest_contents, &dep_path);
        dep.lockfile(&flox).unwrap();

        // Create composer
        let composer_manifest_contents = indoc! {r#"
            version = 1
            [include]
            environments = [
              { dir = "dep" },
            ]
            "#};
        let composer_path = tempdir.path();
        let mut composer =
            new_path_environment_in(&flox, composer_manifest_contents, composer_path);
        let lockfile: Lockfile = composer.lockfile(&flox).unwrap().into();

        let manifest = lockfile.migrated_manifest().unwrap();
        assert_eq!(manifest.as_latest_schema().vars.inner()["foo"], "v1");

        // Call include_upgrade() with a name of an included environment that does not exist
        let err = composer
            .include_upgrade(&flox, vec!["does_not_exist".to_string()])
            .unwrap_err();

        let EnvironmentError::Recoverable(RecoverableMergeError::Catchall(message)) = err else {
            panic!("expected Catchall error, got: {:?}", err)
        };

        assert_eq!(
            message,
            "unknown included environment to check for changes 'does_not_exist'"
        );
    }

    /// An include upgrade that changes nothing doesn't write the lockfile,
    /// so it doesn't rebuild the environment,
    /// or create a generation for managed environments.
    #[test]
    fn include_upgrade_without_changes_does_not_write_lockfile() {
        let (flox, tempdir) = flox_instance();

        let dep_path = tempdir.path().join("dep");
        fs::create_dir(&dep_path).unwrap();
        let mut dep = new_path_environment_in(
            &flox,
            indoc! {r#"
                version = 1
                [vars]
                foo = "v1"
            "#},
            &dep_path,
        );
        dep.lockfile(&flox).unwrap();

        let mut composer = new_path_environment_in(
            &flox,
            indoc! {r#"
                version = 1
                [include]
                environments = [
                  { dir = "dep" },
                ]
            "#},
            tempdir.path(),
        );
        let lockfile: Lockfile = composer.lockfile(&flox).unwrap().into();

        let result = composer.include_upgrade(&flox, vec![]).unwrap();

        assert_eq!(result, UpgradeResult {
            old_lockfile: Some(lockfile.clone()),
            new_lockfile: lockfile,
            store_path: None,
        });
    }

    #[test]
    fn no_rebuild_on_lockfile_formatting_change() {
        let (flox, _temp_dir) = flox_instance();

        let mut environment = new_path_environment(&flox, "version = 1");
        let lockfile: Lockfile = environment.lockfile(&flox).unwrap().into();

        assert!(environment.needs_rebuild(&lockfile));

        environment.rendered_env_links(&flox).unwrap();

        assert!(!environment.needs_rebuild(&lockfile));

        let mut lockfile_file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(environment.lockfile_path(&flox).unwrap())
            .unwrap();

        writeln!(lockfile_file, "\n\n\n",).unwrap();

        let lockfile = environment.existing_lockfile(&flox).unwrap().unwrap();
        assert!(!environment.needs_rebuild(&lockfile));
    }

    // -------------------------------------------------------------------------
    // Regression tests: PathEnvironment::build must not rewrite a current lock
    //
    // Previously, `build` called `lock()` unconditionally.  Even when the
    // lockfile was already up-to-date, `lock()` could rewrite it (e.g. due to
    // formatting normalisation), updating the mtime and triggering a spurious
    // needs_rebuild() on the next activate.
    //
    // After the fix, `build` calls `ensure_locked()`, which skips the catalog
    // round-trip and the file write when lockfile_if_up_to_date() returns
    // Some(_).  These tests assert the "no rewrite" invariant on three
    // environment shapes.
    // -------------------------------------------------------------------------

    /// Plain environment (latest schema, no packages).
    /// After locking once, calling build() must not alter manifest.lock.
    #[test]
    fn build_does_not_rewrite_current_lockfile_plain() {
        let (flox, _temp_dir) = flox_instance();
        let manifest = with_latest_schema("");
        let mut env = new_path_environment(&flox, &manifest);

        // Lock the environment to produce an up-to-date manifest.lock.
        env.lockfile(&flox).unwrap();

        let lockfile_path = env.lockfile_path(&flox).unwrap();
        let bytes_before = fs::read(&lockfile_path).unwrap();
        let mtime_before = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        // Build must not rewrite the lockfile.
        env.build(&flox).unwrap();

        let bytes_after = fs::read(&lockfile_path).unwrap();
        let mtime_after = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        assert_eq!(bytes_before, bytes_after, "lockfile bytes changed");
        assert_eq!(mtime_before, mtime_after, "lockfile mtime changed");
    }

    /// Composed environment (v1 composer that includes a child environment).
    /// After locking once, calling build() on the composer must not alter its
    /// manifest.lock.
    #[test]
    fn build_does_not_rewrite_current_lockfile_composed() {
        let (flox, tempdir) = flox_instance();

        // Set up an included environment with only vars (backwards compatible
        // with v1, so the composer won't be migrated).
        let included_manifest = with_latest_schema(indoc! {r#"
            [vars]
            included_var = "value"
        "#});
        let included_path = tempdir.path().join("included");
        let mut included_env = new_path_environment_in(&flox, &included_manifest, &included_path);
        included_env.lockfile(&flox).unwrap();

        // Create a v1 composer that includes the child.
        let composer_manifest = with_schema(KnownSchemaVersion::V1, indoc! {r#"
            [include]
            environments = [
              { dir = "../included" },
            ]
        "#});
        let composer_path = tempdir.path().join("composer");
        let mut composer = new_path_environment_in(&flox, &composer_manifest, &composer_path);

        // Lock the composer to produce an up-to-date manifest.lock.
        composer.lockfile(&flox).unwrap();

        let lockfile_path = composer.lockfile_path(&flox).unwrap();
        let bytes_before = fs::read(&lockfile_path).unwrap();
        let mtime_before = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        // Build must not rewrite the lockfile.
        composer.build(&flox).unwrap();

        let bytes_after = fs::read(&lockfile_path).unwrap();
        let mtime_after = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        assert_eq!(bytes_before, bytes_after, "lockfile bytes changed");
        assert_eq!(mtime_before, mtime_after, "lockfile mtime changed");
    }

    /// Create `<tempdir>/<name>` with `contents` and lock it
    fn locked_path_environment(
        flox: &Flox,
        tempdir: &TempDir,
        name: &str,
        contents: &str,
    ) -> PathEnvironment {
        let mut environment = new_path_environment_in(flox, contents, tempdir.path().join(name));
        environment.lockfile(flox).unwrap();
        environment
    }

    /// Replace the manifest of an environment and lock it, like 'flox edit'
    /// without building
    fn edit_and_lock(environment: &mut PathEnvironment, flox: &Flox, contents: &str) {
        fs::write(environment.manifest_path(flox).unwrap(), contents).unwrap();
        environment
            .as_core_environment_mut()
            .unwrap()
            .ensure_locked(flox)
            .unwrap();
    }

    /// The vars of the merged manifest in a lockfile
    fn locked_vars(lockfile: &Lockfile) -> BTreeMap<String, String> {
        lockfile
            .migrated_manifest()
            .unwrap()
            .as_latest_schema()
            .vars
            .inner()
            .clone()
    }

    fn vars_map(vars: &[(&str, &str)]) -> BTreeMap<String, String> {
        vars.iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    /// The names in [FollowedIncludes], as
    /// (unsaved, unreadable, not locked, not built),
    /// since the reasons can't be compared
    #[allow(clippy::type_complexity)]
    fn followed_names(followed: &FollowedIncludes) -> (Vec<&str>, Vec<&str>, Vec<&str>, Vec<&str>) {
        fn names(not_applied: &Option<NotAppliedIncludes>) -> Vec<&str> {
            not_applied
                .iter()
                .flat_map(|not_applied| not_applied.names.iter().map(String::as_str))
                .collect()
        }
        (
            followed.unsaved.iter().map(String::as_str).collect(),
            followed
                .unreadable
                .iter()
                .map(|unreadable| unreadable.name.as_str())
                .collect(),
            names(&followed.not_locked),
            names(&followed.not_built),
        )
    }

    /// Follow included environments and return the lockfile in use,
    /// asserting that the environment's own lockfile isn't written
    fn follow(
        environment: &mut PathEnvironment,
        flox: &Flox,
        mode: FollowMode,
    ) -> (Lockfile, FollowedIncludes) {
        let lockfile_path = environment.lockfile_path(flox).unwrap();
        let bytes_before = fs::read(&lockfile_path).unwrap();
        let (lock_result, followed) = environment.lockfile_following_includes(flox, mode).unwrap();
        assert_eq!(
            bytes_before,
            fs::read(&lockfile_path).unwrap(),
            "following included environments wrote the lockfile"
        );
        (lock_result.into(), followed)
    }

    /// Locked changes to an included path environment are used without
    /// 'flox include upgrade', and without writing the lockfile.
    #[test]
    fn lockfile_follows_locked_changes_to_path_include() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        // A v1 composer, like most composers in the wild
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_schema(
                KnownSchemaVersion::V1,
                "[include]\nenvironments = [{ dir = \"../included\" }]",
            ),
        );

        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
        assert_eq!(locked_vars(&lockfile), vars_map(&[("included", "v2")]));
        // Locking alone doesn't build the copy
        let copy_build = |composer: &PathEnvironment| {
            composer
                .read_followed_lockfile(&flox.system)
                .map(|copy| copy.build)
        };
        assert_eq!(copy_build(&composer), Some(FollowedBuild::Pending));
        follow(&mut composer, &flox, FollowMode::LockAndBuild);
        assert_eq!(copy_build(&composer), Some(FollowedBuild::Succeeded));

        // The copy is reused, and the changes are still unsaved
        let (lock_result, followed) = composer
            .lockfile_following_includes(&flox, FollowMode::Lock)
            .unwrap();
        assert!(matches!(lock_result, LockResult::Unchanged(_)));
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
    }

    /// Changes that an included environment hasn't locked aren't used.
    #[test]
    fn lockfile_does_not_follow_unlocked_changes_to_path_include() {
        let (flox, tempdir) = flox_instance();
        let included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );

        fs::write(
            included.manifest_path(&flox).unwrap(),
            with_latest_schema("[vars]\nincluded = \"v2\""),
        )
        .unwrap();

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec![], vec!["included"], vec![], vec![])
        );
        assert!(matches!(
            followed.unreadable[0].reason.as_ref(),
            EnvironmentError::Recoverable(RecoverableMergeError::PathOutOfSync(_))
        ));
        assert_eq!(locked_vars(&lockfile), vars_map(&[("included", "v1")]));
    }

    /// An included environment that can't be read keeps the version in use,
    /// rather than going back to the version in the lockfile.
    #[test]
    fn lockfile_keeps_version_in_use_of_path_include_that_cannot_be_read() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );
        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );
        follow(&mut composer, &flox, FollowMode::Lock);

        fs::remove_dir_all(tempdir.path().join("included")).unwrap();

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec!["included"], vec![], vec![])
        );
        assert!(matches!(
            followed.unreadable[0].reason.as_ref(),
            EnvironmentError::DotFloxNotFound(_)
        ));
        assert_eq!(locked_vars(&lockfile), vars_map(&[("included", "v2")]));
    }

    /// A -> B -> C: a change that C locked reaches A through B,
    /// without B locking it.
    #[test]
    fn lockfile_follows_locked_changes_to_nested_path_include() {
        let (flox, tempdir) = flox_instance();
        let mut c = locked_path_environment(
            &flox,
            &tempdir,
            "c",
            &with_latest_schema("[vars]\nc = \"v1\""),
        );
        let b = locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema(
                "[vars]\nb = \"v1\"\n[include]\nenvironments = [{ dir = \"../c\" }]",
            ),
        );
        let mut a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );
        let b_lockfile_before = fs::read(b.lockfile_path(&flox).unwrap()).unwrap();

        edit_and_lock(&mut c, &flox, &with_latest_schema("[vars]\nc = \"v2\""));

        let (lockfile, followed) = follow(&mut a, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["b"], vec![], vec![], vec![])
        );
        assert_eq!(
            locked_vars(&lockfile),
            vars_map(&[("b", "v1"), ("c", "v2")])
        );
        assert_eq!(
            b_lockfile_before,
            fs::read(b.lockfile_path(&flox).unwrap()).unwrap()
        );
    }

    /// The lockfile that the rendered environment links point to
    fn rendered_lockfile(environment: &PathEnvironment) -> Lockfile {
        let path = environment.rendered_env_links.dev.join(LOCKFILE_FILENAME);
        Lockfile::read_from_file(&CanonicalPath::new(path).unwrap()).unwrap()
    }

    /// A command that changes the lockfile keeps the latest changes to
    /// followed includes in the rendered environment, which it rebuilt.
    #[test]
    fn edit_keeps_followed_changes_in_rendered_environment() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let include = "[include]\nenvironments = [{ dir = \"../included\" }]";
        let mut composer =
            locked_path_environment(&flox, &tempdir, "composer", &with_latest_schema(include));
        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );
        follow(&mut composer, &flox, FollowMode::LockAndBuild);

        composer
            .edit(
                &flox,
                with_latest_schema(format!("[vars]\ncomposer = \"v1\"\n{include}")),
            )
            .unwrap();
        assert_eq!(
            locked_vars(&rendered_lockfile(&composer)),
            vars_map(&[("composer", "v1"), ("included", "v2")])
        );
    }

    /// An include cycle in an existing lockfile, which earlier versions could
    /// lock, keeps the version in use rather than failing or growing.
    #[test]
    fn lockfile_keeps_path_include_that_became_a_cycle() {
        let (flox, tempdir) = flox_instance();
        locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );

        // The include now resolves to the composer itself
        fs::remove_dir_all(tempdir.path().join("included")).unwrap();
        std::os::unix::fs::symlink(
            tempdir.path().join("composer"),
            tempdir.path().join("included"),
        )
        .unwrap();

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec![], vec!["included"], vec![], vec![])
        );
        assert!(matches!(
            followed.unreadable[0].reason.as_ref(),
            EnvironmentError::Recoverable(RecoverableMergeError::IncludeCycle(_))
        ));
        assert_eq!(locked_vars(&lockfile), vars_map(&[("included", "v1")]));
    }

    /// Adding an include that closes a cycle is an error, even though the
    /// environment it includes is locked with the other half of the cycle.
    #[test]
    fn lockfile_errors_on_new_include_cycle() {
        let (flox, tempdir) = flox_instance();
        let mut b = locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema("[vars]\nb = \"v1\""),
        );
        let a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );

        fs::write(
            b.manifest_path(&flox).unwrap(),
            with_latest_schema("[include]\nenvironments = [{ dir = \"../a\" }]"),
        )
        .unwrap();

        let err = b.lockfile(&flox).unwrap_err();
        let EnvironmentError::Recoverable(RecoverableMergeError::Fetch { err, .. }) = err else {
            panic!("expected fetching a to fail, got: {err:?}");
        };
        let EnvironmentError::Recoverable(RecoverableMergeError::IncludeCycle(cycle)) = *err else {
            panic!("expected an include cycle, got: {err:?}");
        };
        assert_eq!(cycle, vec![
            b.dot_flox_path().to_path_buf(),
            a.dot_flox_path().to_path_buf(),
            b.dot_flox_path().to_path_buf(),
        ]);
    }

    /// Changes that can't be locked together keep the versions in use,
    /// here because a renamed included environment takes another's name.
    #[test]
    fn lockfile_keeps_versions_in_use_when_changes_cannot_be_locked() {
        let (flox, tempdir) = flox_instance();
        let mut included1 = locked_path_environment(
            &flox,
            &tempdir,
            "included1",
            &with_latest_schema("[vars]\nincluded1 = \"v1\""),
        );
        locked_path_environment(
            &flox,
            &tempdir,
            "included2",
            &with_latest_schema("[vars]\nincluded2 = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema(indoc! {r#"
                [include]
                environments = [{ dir = "../included1" }, { dir = "../included2" }]
            "#}),
        );

        included1.rename("included2".parse().unwrap()).unwrap();

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec![], vec![], vec!["included1"], vec![])
        );
        assert_eq!(
            locked_vars(&lockfile),
            vars_map(&[("included1", "v1"), ("included2", "v1")])
        );
    }

    /// Changes that lock but don't build, here a cycle between the vars of the
    /// composer and an included environment, fall back to the lockfile,
    /// which is remembered until they change.
    #[test]
    fn lockfile_falls_back_to_lockfile_when_changes_do_not_build() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\ny = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema(indoc! {r#"
                [vars]
                x = "$y"
                [include]
                environments = [{ dir = "../included" }]
            "#}),
        );

        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\ny = \"$x\""),
        );

        for mode in [FollowMode::LockAndBuild, FollowMode::Lock] {
            let (lockfile, followed) = follow(&mut composer, &flox, mode);
            assert!(matches!(
                composer
                    .read_followed_lockfile(&flox.system)
                    .map(|copy| copy.build),
                Some(FollowedBuild::Failed(_))
            ));
            assert_eq!(
                followed_names(&followed),
                (vec![], vec![], vec![], vec!["included"])
            );
            assert_eq!(
                locked_vars(&lockfile),
                vars_map(&[("x", "$y"), ("y", "v1")])
            );
        }
    }

    /// Changes that need a newer schema than the composer's are used without
    /// touching the composer's manifest, since only the copy has them.
    #[test]
    fn lockfile_follows_changes_that_require_newer_schema() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_schema(KnownSchemaVersion::V1_10_0, "[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_schema(
                KnownSchemaVersion::V1_10_0,
                "[include]\nenvironments = [{ dir = \"../included\" }]",
            ),
        );
        let manifest_path = composer.manifest_path(&flox).unwrap();
        let manifest_before = fs::read(&manifest_path).unwrap();

        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema(indoc! {r#"
                [vars]
                included = "v2"
                [options.activate]
                upgrade-notifications = false
            "#}),
        );

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
        assert_eq!(locked_vars(&lockfile), vars_map(&[("included", "v2")]));
        assert_eq!(manifest_before, fs::read(&manifest_path).unwrap());
    }

    /// 'flox include upgrade' fails rather than saving older versions of
    /// nested includes than the ones in use, and keeps the copy in use.
    #[test]
    fn include_upgrade_fails_when_nested_include_cannot_be_read() {
        let (flox, tempdir) = flox_instance();
        let mut c = locked_path_environment(
            &flox,
            &tempdir,
            "c",
            &with_latest_schema("[vars]\nc = \"v1\""),
        );
        locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../c\" }]"),
        );
        let mut a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );
        edit_and_lock(&mut c, &flox, &with_latest_schema("[vars]\nc = \"v2\""));
        follow(&mut a, &flox, FollowMode::Lock);

        fs::write(
            c.manifest_path(&flox).unwrap(),
            with_latest_schema("[vars]\nc = \"v3\""),
        )
        .unwrap();

        a.include_upgrade(&flox, vec![]).unwrap_err();
        let (lockfile, followed) = follow(&mut a, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["b"], vec!["b"], vec![], vec![])
        );
        assert_eq!(locked_vars(&lockfile), vars_map(&[("c", "v2")]));
    }

    /// A -> B -> C: when C can't be read anymore, A keeps B with the version
    /// of C in use, rather than the older one in B's lockfile.
    #[test]
    fn lockfile_keeps_version_in_use_when_nested_include_cannot_be_read() {
        let (flox, tempdir) = flox_instance();
        let mut c = locked_path_environment(
            &flox,
            &tempdir,
            "c",
            &with_latest_schema("[vars]\nc = \"v1\""),
        );
        locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../c\" }]"),
        );
        let mut a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );
        edit_and_lock(&mut c, &flox, &with_latest_schema("[vars]\nc = \"v2\""));
        follow(&mut a, &flox, FollowMode::Lock);

        fs::remove_dir_all(tempdir.path().join("c")).unwrap();

        let (lockfile, followed) = follow(&mut a, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["b"], vec!["b"], vec![], vec![])
        );
        assert_eq!(locked_vars(&lockfile), vars_map(&[("c", "v2")]));
    }

    /// New changes are added to the changes already in use.
    #[test]
    fn lockfile_adds_new_changes_to_changes_in_use() {
        let (flox, tempdir) = flox_instance();
        let mut included1 = locked_path_environment(
            &flox,
            &tempdir,
            "included1",
            &with_latest_schema("[vars]\nincluded1 = \"v1\""),
        );
        let mut included2 = locked_path_environment(
            &flox,
            &tempdir,
            "included2",
            &with_latest_schema("[vars]\nincluded2 = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema(indoc! {r#"
                [include]
                environments = [{ dir = "../included1" }, { dir = "../included2" }]
            "#}),
        );

        edit_and_lock(
            &mut included1,
            &flox,
            &with_latest_schema("[vars]\nincluded1 = \"v2\""),
        );
        follow(&mut composer, &flox, FollowMode::Lock);
        edit_and_lock(
            &mut included2,
            &flox,
            &with_latest_schema("[vars]\nincluded2 = \"v2\""),
        );

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included1", "included2"], vec![], vec![], vec![])
        );
        assert_eq!(
            locked_vars(&lockfile),
            vars_map(&[("included1", "v2"), ("included2", "v2")])
        );
    }

    /// When the lockfile changes, the changes in use are copied again on top
    /// of it.
    #[test]
    fn lockfile_copies_changes_in_use_again_after_lockfile_changes() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );
        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );
        follow(&mut composer, &flox, FollowMode::Lock);

        edit_and_lock(
            &mut composer,
            &flox,
            &with_latest_schema(indoc! {r#"
                [vars]
                composer = "v2"
                [include]
                environments = [{ dir = "../included" }]
            "#}),
        );

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
        assert_eq!(
            locked_vars(&lockfile),
            vars_map(&[("composer", "v2"), ("included", "v2")])
        );
    }

    /// Building uses the changes in use, while publishing builds the
    /// lockfile.
    #[test]
    fn build_uses_changes_in_use_and_build_locked_does_not() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );
        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );
        let (in_use, _) = follow(&mut composer, &flox, FollowMode::Lock);
        let rendered_lockfile = |composer: &PathEnvironment| {
            Lockfile::read_from_file(
                &CanonicalPath::new(composer.rendered_env_links.dev.join(LOCKFILE_FILENAME))
                    .unwrap(),
            )
            .unwrap()
        };

        composer.build(&flox).unwrap();
        assert_eq!(rendered_lockfile(&composer), in_use);

        composer.build_locked(&flox).unwrap();
        assert_eq!(
            Some(rendered_lockfile(&composer)),
            composer.existing_lockfile(&flox).unwrap()
        );
    }

    /// 'flox include upgrade' saves the changes that were in use to the
    /// lockfile.
    #[test]
    fn include_upgrade_saves_followed_changes() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );
        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );
        let (followed_lockfile, _) = follow(&mut composer, &flox, FollowMode::Lock);

        composer.include_upgrade(&flox, vec![]).unwrap();
        assert!(!composer.followed_lockfile_path(&flox.system).exists());

        // The lockfile gets exactly what was in use, not a new lock of it
        let saved = composer.existing_lockfile(&flox).unwrap().unwrap();
        assert_eq!(saved, followed_lockfile);
        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(followed_names(&followed), (vec![], vec![], vec![], vec![]));
        assert_eq!(lockfile, saved);
    }

    /// v1 schema environment (no packages).
    /// After locking once, calling build() must not alter manifest.lock.
    /// Exercises the v1 → ensure_locked path in lockfile_if_up_to_date.
    #[test]
    fn build_does_not_rewrite_current_lockfile_v1() {
        let (flox, _temp_dir) = flox_instance();
        let manifest = with_schema(KnownSchemaVersion::V1, "");
        let mut env = new_path_environment(&flox, &manifest);

        // Lock the environment to produce an up-to-date manifest.lock.
        env.lockfile(&flox).unwrap();

        let lockfile_path = env.lockfile_path(&flox).unwrap();
        let bytes_before = fs::read(&lockfile_path).unwrap();
        let mtime_before = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        // Build must not rewrite the lockfile.
        env.build(&flox).unwrap();

        let bytes_after = fs::read(&lockfile_path).unwrap();
        let mtime_after = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        assert_eq!(bytes_before, bytes_after, "lockfile bytes changed");
        assert_eq!(mtime_before, mtime_after, "lockfile mtime changed");
    }

    // -------------------------------------------------------------------------
    // Cross-release coverage: a current-release activate must accept the build
    // stamp left by an earlier release without triggering a needless rebuild.
    //
    // Fixtures live in test_data/manually_generated/prior_release_baselines/
    // and are captured by the `regen-prior-release-fixtures` Justfile recipe.
    // -------------------------------------------------------------------------

    /// Building a prior-release manifest + lockfile with the current release
    /// and then checking `needs_rebuild()` must return false: the current
    /// release accepts the prior-release build state without rebuilding.
    #[test]
    fn needs_rebuild_accepts_prior_release_stamp_plain() {
        use flox_test_utils::MANUALLY_GENERATED;

        let (flox, _temp_dir) = flox_instance();

        let base = MANUALLY_GENERATED
            .join("prior_release_baselines")
            .join("plain");

        // new_path_environment_from_env_files reads manifest.toml and
        // manifest.lock from the given directory and writes them into a
        // fresh .flox/env/ directory, giving us a PathEnvironment with the
        // prior-release lockfile already in place.
        let mut env = new_path_environment_from_env_files(&flox, &base);

        // Before the first build, the rendered-env link does not exist, so
        // needs_rebuild() returns true — the env needs to be built once.
        let lockfile = env.existing_lockfile(&flox).unwrap().unwrap();
        assert!(
            env.needs_rebuild(&lockfile),
            "needs_rebuild() should return true before first build"
        );

        // Lock (no-op if the prior-release lockfile is up-to-date) and
        // build, creating the rendered-env stamp.
        env.build(&flox)
            .expect("build should succeed with the prior-release lockfile");

        // After the build, needs_rebuild() must return false: the
        // rendered-env stamp's lockfile matches the env's lockfile.
        assert!(
            !env.needs_rebuild(&lockfile),
            "needs_rebuild() returned true after building from a prior-release \
             lockfile: the rendered-env stamp's lockfile diverged from the \
             env's lockfile across releases",
        );
    }
}
