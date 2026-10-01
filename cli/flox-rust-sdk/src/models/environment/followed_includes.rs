//! Following included environments: using the latest changes that the
//! included environments of a local environment locked, from a copy of its
//! lockfile in `.flox/cache`, without writing its lockfile.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, iter};

use flox_core::{blake3_hex, write_atomically};
use flox_manifest::lockfile::{LockedInclude, Lockfile};
use flox_manifest::parsed::latest::AutoUpgrade;
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
};
use super::fetcher::IncludeFetcher;
use super::{CACHE_DIR_NAME, EnvironmentError, RenderedEnvironmentLinks};
use crate::data::System;
use crate::flox::Flox;
use crate::providers::buildenv::{BuildEnvError, BuildEnvOutputs};
use crate::providers::lock_manifest::LockResult;

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
pub(super) struct FollowedLockfile {
    /// Hash of the lockfile that this is a copy of
    base: String,
    pub(super) lockfile: Lockfile,
    pub(super) build: FollowedBuild,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum FollowedBuild {
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
pub(super) struct Following {
    pub(super) lock_result: LockResult,
    pub(super) followed: FollowedIncludes,
    /// The copy of the lockfile in use, if any
    pub(super) copy: Option<FollowedLockfile>,
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

/// Hash of a lockfile, which a copy of it is keyed to
fn lockfile_hash(lockfile: &Lockfile) -> String {
    blake3_hex(&serde_json::to_vec(lockfile).expect("lockfile is valid json"))
}

/// The copies of an environment's lockfile in its `.flox/cache`
#[derive(Clone, Debug)]
pub(super) struct FollowedLockfiles {
    cache_dir: PathBuf,
}

impl FollowedLockfiles {
    /// The copies for the environment whose `.flox` directory is `dot_flox`
    pub(super) fn in_dot_flox(dot_flox: &Path) -> Self {
        Self {
            cache_dir: dot_flox.join(CACHE_DIR_NAME),
        }
    }

    fn path(&self, system: &System) -> PathBuf {
        self.cache_dir
            .join(format!("{FOLLOWED_LOCKFILE_PREFIX}{system}.json"))
    }

    pub(super) fn read(&self, system: &System) -> Option<FollowedLockfile> {
        let contents = fs::read_to_string(self.path(system)).ok()?;
        serde_json::from_str(&contents)
            .inspect_err(|err| debug!(%err, "ignoring unreadable copy of the lockfile"))
            .ok()
    }

    /// The copy of `committed` for `system` that commands use, if any
    pub(super) fn in_use(&self, committed: &Lockfile, system: &System) -> Option<FollowedLockfile> {
        let base = lockfile_hash(committed);
        self.read(system)
            .filter(|copy| copy.base == base && !copy.build.failed())
    }

    /// Failing to keep the copy only means that it's made again next time.
    fn write(&self, system: &System, copy: &FollowedLockfile) {
        let contents = serde_json::to_string(copy).expect("lockfile is valid json");
        if let Err(err) = fs::create_dir_all(&self.cache_dir) {
            debug!(%err, "could not keep copy of the lockfile");
            return;
        }
        if let Err(err) = write_atomically(self.path(system), contents) {
            debug!(%err, "could not keep copy of the lockfile");
        }
    }

    fn remove(&self, system: &System) {
        let path = self.path(system);
        if path.exists()
            && let Err(err) = fs::remove_file(&path)
        {
            debug!(%err, "could not remove copy of the lockfile");
        }
    }

    /// Remove the copies of the lockfile for every system
    pub(super) fn remove_all(&self) {
        let Ok(entries) = fs::read_dir(&self.cache_dir) else {
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
}

/// Use a copy of the lockfile that `lock_result` locked, with the latest
/// changes to the included environments that `env_view` follows if any
/// changed.
///
/// The `auto-upgrade` field of each include decides whether it's
/// followed, see [AutoUpgrade].
/// Only changes that the included environments have locked are used.
/// Changes that can't be read or locked keep the versions in use before,
/// and changes that don't build together with this environment fall back
/// to the environment's lockfile, as [FollowedIncludes] reports.
pub(super) fn follow_includes(
    env_view: &mut CoreEnvironment,
    lock_result: LockResult,
    copies: &FollowedLockfiles,
    rendered_env_links: &RenderedEnvironmentLinks,
    flox: &Flox,
    mode: FollowMode,
) -> Result<Following, EnvironmentError> {
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
        copies.remove(&flox.system);
        return Ok(locked_without_copy(FollowedIncludes::default()));
    }

    let base = lockfile_hash(committed);
    let cached = copies
        .read(&flox.system)
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
        copies.remove(&flox.system);
        return Ok(locked_without_copy(followed));
    };
    if is_new {
        copies.write(&flox.system, &copy);
    }
    let unsaved = changed_include_names(committed, &copy.lockfile);

    if mode == FollowMode::LockAndBuild
        && copy.build == FollowedBuild::Pending
        && let Err(err) =
            build_followed_lockfile(env_view, copies, rendered_env_links, flox, &mut copy)
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

/// Build a copy of the lockfile with the latest changes to included
/// environments into the rendered environment links,
/// and record whether that worked.
pub(super) fn build_followed_lockfile(
    env_view: &mut CoreEnvironment,
    copies: &FollowedLockfiles,
    rendered_env_links: &RenderedEnvironmentLinks,
    flox: &Flox,
    copy: &mut FollowedLockfile,
) -> Result<BuildEnvOutputs, EnvironmentError> {
    let out_link_prefix = rendered_env_links.out_link_prefix();
    let result = env_view.build_lockfile(flox, &copy.lockfile, Some(out_link_prefix));
    let build = match &result {
        Ok(_) => FollowedBuild::Succeeded,
        Err(err) if fails_every_time(err) => FollowedBuild::Failed(error_chain(err)),
        Err(_) => FollowedBuild::Pending,
    };
    if copy.build != build {
        copy.build = build;
        copies.write(&flox.system, copy);
    }
    let outputs = result?;
    rendered_env_links.replace_legacy_links();
    Ok(outputs)
}

/// Names of the included environments that the copy of `committed` in use
/// has changes to, which `committed` doesn't have yet.
///
/// Unlike [follow_includes], this doesn't lock anything,
/// so it reports the copy from the last command that used it.
pub(super) fn unsaved_followed_includes(
    committed: &Lockfile,
    copies: &FollowedLockfiles,
    system: &System,
) -> Vec<String> {
    copies
        .in_use(committed, system)
        .map(|copy| changed_include_names(committed, &copy.lockfile))
        .unwrap_or_default()
}

/// The copy of `committed` in use that 'flox include upgrade' of
/// `to_upgrade` saves, rather than locking the changes again,
/// so that the lockfile gets what was used.
///
/// There's none if it has changes to includes that weren't named.
pub(super) fn copy_to_save(
    committed: &Lockfile,
    copies: &FollowedLockfiles,
    system: &System,
    to_upgrade: &[String],
) -> Option<FollowedLockfile> {
    copies.in_use(committed, system).filter(|copy| {
        to_upgrade.is_empty()
            || changed_include_names(committed, &copy.lockfile)
                .iter()
                .all(|name| to_upgrade.contains(name))
    })
}

/// Check whether `committed`, the lockfile of `env_view`, has the latest
/// changes to the included environments that are followed, without writing
/// anything.
///
/// Unlike following, this doesn't use or keep a copy of the lockfile, and it
/// always builds the latest changes, without replacing the rendered
/// environment links, so it reports the same wherever it runs.
/// Returns [None] if the lockfile isn't up to date with the manifest.
pub(super) fn check_followed_includes(
    env_view: &mut CoreEnvironment,
    committed: Option<Lockfile>,
    flox: &Flox,
) -> Result<Option<FollowedIncludes>, EnvironmentError> {
    let Some(committed) = committed else {
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

    let lockfile = match env_view.lock_with_latest_includes(flox, &committed, check.changed.clone())
    {
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

/// Whether the included environment named `name` in `lockfile` is followed,
/// so its locked changes are used without 'flox include upgrade'.
///
/// An include whose environment can't be opened counts as not followed.
pub(super) fn follows_include_named(
    lockfile: Option<&Lockfile>,
    include_fetcher: &IncludeFetcher,
    name: &str,
) -> bool {
    let Some(locked) = lockfile
        .and_then(locked_includes)
        .into_iter()
        .flatten()
        .find(|locked| locked.name == name)
    else {
        return false;
    };
    include_fetcher
        .is_auto_upgraded(&locked.descriptor)
        .unwrap_or_else(|err| {
            debug!(include = name, %err, "could not tell whether the include is followed");
            false
        })
}
