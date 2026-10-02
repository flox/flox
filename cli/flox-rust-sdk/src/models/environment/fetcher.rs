use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};

use flox_core::data::environment_ref::RemoteEnvironmentRef;
use flox_manifest::lockfile::{IncludedRemote, LOCKFILE_FILENAME, LockedInclude, Lockfile};
use flox_manifest::parsed::latest::{AutoUpgrade, IncludeDescriptor};
use flox_manifest::{MANIFEST_FILENAME, Manifest, TypedOnly};
use itertools::Itertools;

use super::core_environment::{CoreEnvironment, IncludedRemoteEnvironment};
use super::{
    ConcreteEnvironment,
    DotFlox,
    ENV_DIR_NAME,
    EnvironmentError,
    EnvironmentPointer,
    UninitializedEnvironment,
};
use crate::data::CanonicalPath;
use crate::flox::Flox;
use crate::models::environment::core_environment::CoreEnvironmentError;
use crate::models::environment::floxmeta_branch::{
    fetch_remote_generations,
    last_fetched_remote_generations,
    local_generations,
};
use crate::models::environment::generations::GenerationsError;
use crate::models::environment::managed_environment::{
    ManagedEnvironment,
    ManagedEnvironmentError,
};
use crate::models::environment::{Environment, ManagedPointer, UnreadableIncludes};
use crate::providers::lock_manifest::RecoverableMergeError;

/// Included environments that have been fetched.
///
/// Sharing them between fetchers fetches each included environment once,
/// even when several environments include it, so everything that uses it
/// sees the same version.
/// Nothing a command writes is fetched again by the same command,
/// since a command only writes the environment that includes the others.
#[derive(Clone, Debug, Default)]
pub struct FetchedIncludes(Arc<Mutex<FetchedIncludesInner>>);

#[derive(Debug, Default)]
struct FetchedIncludesInner {
    /// The lockfiles of remote environments and the generations they're
    /// from, by environment and requested generation
    remote: HashMap<RemoteKey, (Lockfile, usize)>,
    /// Local environments, by `.flox` directory and how their own
    /// unreadable includes were handled
    local: HashMap<(PathBuf, UnreadableIncludes), LocalFetch>,
    /// The `.flox` directories of the local environments fetched so far,
    /// in order, so that a fetch can tell which ones it fetched in turn
    fetch_log: Vec<PathBuf>,
}

/// A local environment as fetched
#[derive(Clone, Debug)]
struct LocalFetch {
    name: String,
    manifest: Manifest<TypedOnly>,
    lockfile: Lockfile,
    /// The `.flox` directories of the environment and of every environment
    /// fetched for it in turn.
    ///
    /// Whether fetching finds an include cycle depends on the environments
    /// that include it, so the fetch is only reused while none of these are
    /// among them.
    subtree: Vec<PathBuf>,
}

impl FetchedIncludes {
    fn lock(&self) -> MutexGuard<'_, FetchedIncludesInner> {
        self.0
            .lock()
            .expect("fetched includes lock should not be poisoned")
    }

    fn remote(&self, key: &RemoteKey) -> Option<(Lockfile, usize)> {
        self.lock().remote.get(key).cloned()
    }

    fn insert_remote(&self, key: RemoteKey, fetched: (Lockfile, usize)) {
        self.lock().remote.insert(key, fetched);
    }

    /// A local environment fetched before, unless one of `composers` is in
    /// its subtree, which would make fetching it again find a cycle
    fn local(
        &self,
        key: &(PathBuf, UnreadableIncludes),
        composers: &[CanonicalPath],
    ) -> Option<LocalFetch> {
        let mut inner = self.lock();
        let fetched = inner.local.get(key)?.clone();
        if fetched
            .subtree
            .iter()
            .any(|dot_flox| composers.iter().any(|composer| **composer == *dot_flox))
        {
            return None;
        }
        inner.fetch_log.extend(fetched.subtree.iter().cloned());
        Some(fetched)
    }

    /// Record that fetching the local environment at `dot_flox` started,
    /// returning where its subtree starts in the fetch log
    fn start_local(&self, dot_flox: &Path) -> usize {
        let mut inner = self.lock();
        inner.fetch_log.push(dot_flox.to_path_buf());
        inner.fetch_log.len() - 1
    }

    /// Keep a local environment that was fetched starting at `start`
    fn insert_local(
        &self,
        key: (PathBuf, UnreadableIncludes),
        start: usize,
        name: String,
        manifest: Manifest<TypedOnly>,
        lockfile: Lockfile,
    ) {
        let mut inner = self.lock();
        let subtree = inner.fetch_log[start..].iter().unique().cloned().collect();
        inner.local.insert(key, LocalFetch {
            name,
            manifest,
            lockfile,
            subtree,
        });
    }
}

/// A remote environment that was fetched: the environment, the generation
/// requested, and where it was read from
type RemoteKey = (RemoteEnvironmentRef, Option<usize>, RemoteSource);

/// Where environments included from FloxHub are read from
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RemoteSource {
    /// Fetch them from FloxHub
    FloxHub,
    /// Read them as they were last fetched, without contacting FloxHub,
    /// which the background upgrade check does
    LastFetched,
}

/// Context required to fetch an environment include
#[derive(Clone, Debug)]
pub struct IncludeFetcher {
    pub base_directory: Option<PathBuf>,
    /// The `.flox` directories of the environments whose includes are being
    /// fetched, outermost first, used to detect include cycles
    composers: Vec<CanonicalPath>,
    fetched: FetchedIncludes,
    remote_source: RemoteSource,
}

/// The included environment as fetched,
/// with the lockfile whose packages seed the composing environment's lock.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedInclude {
    pub locked_include: LockedInclude,
    /// The included environment's lockfile.
    ///
    /// For a path environment merged with the latest changes to its own
    /// includes, this is its lockfile with the packages those includes
    /// locked, which may not lock every package in the fetched manifest.
    /// Seeding only reuses packages locked for descriptors that the merged
    /// manifest doesn't change.
    pub lockfile: Lockfile,
}

impl IncludeFetcher {
    pub fn new(base_directory: Option<PathBuf>) -> Self {
        Self {
            base_directory,
            composers: Vec::new(),
            fetched: FetchedIncludes::default(),
            remote_source: RemoteSource::FloxHub,
        }
    }

    /// A fetcher for the includes of the environment at `dot_flox`,
    /// whose relative include directories are resolved against
    /// `base_directory`
    pub fn for_composer(base_directory: PathBuf, dot_flox: CanonicalPath) -> Self {
        Self {
            base_directory: Some(base_directory),
            composers: vec![dot_flox],
            fetched: FetchedIncludes::default(),
            remote_source: RemoteSource::FloxHub,
        }
    }

    /// A fetcher that reads included environments from FloxHub as they were
    /// last fetched, without contacting FloxHub.
    ///
    /// Commands follow included environments with it, so following never
    /// needs the network.
    /// Reading one that hasn't been fetched on this machine yet fails with
    /// [RecoverableMergeError::RemoteNotFetched].
    pub(crate) fn reading_last_fetched_remotes(&self) -> Self {
        Self {
            remote_source: RemoteSource::LastFetched,
            ..self.clone()
        }
    }

    /// Use, and add to, included environments that other fetchers have
    /// fetched
    pub fn with_fetched_includes(mut self, fetched: FetchedIncludes) -> Self {
        self.fetched = fetched;
        self
    }

    /// A fetcher for the includes of an included path environment,
    /// which is merged with them in memory as part of fetching it
    fn for_included(&self, base_directory: PathBuf, dot_flox: CanonicalPath) -> Self {
        let mut composers = self.composers.clone();
        composers.push(dot_flox);
        Self {
            base_directory: Some(base_directory),
            composers,
            fetched: self.fetched.clone(),
            remote_source: self.remote_source,
        }
    }

    /// Fetch an included environment.
    ///
    /// An included path environment whose own automatically upgraded includes
    /// can't be read uses its lockfile's copies of them.
    pub fn fetch(
        &self,
        flox: &Flox,
        include_environment: &IncludeDescriptor,
    ) -> Result<FetchedInclude, EnvironmentError> {
        self.fetch_with(flox, include_environment, UnreadableIncludes::UseLocked)
    }

    /// Fetch the latest version of an included environment.
    ///
    /// Unlike [Self::fetch], this fails if the latest locked changes to a path
    /// environment included below it can't be read.
    pub(crate) fn fetch_latest(
        &self,
        flox: &Flox,
        include_environment: &IncludeDescriptor,
    ) -> Result<FetchedInclude, EnvironmentError> {
        self.fetch_with(flox, include_environment, UnreadableIncludes::Fail)
    }

    /// Whether an included environment's `auto-upgrade` field says to use its
    /// latest changes without 'flox include upgrade'.
    ///
    /// By default that's an included path environment.
    pub fn is_auto_upgraded(
        &self,
        include_environment: &IncludeDescriptor,
    ) -> Result<bool, EnvironmentError> {
        Ok(
            match (include_environment.auto_upgrade(), include_environment) {
                (AutoUpgrade::Never, _) => false,
                (AutoUpgrade::Always, _) => true,
                (AutoUpgrade::IfPathEnvironment, IncludeDescriptor::Local { dir, .. }) => {
                    let path = self
                        .expand_include_dir(dir)
                        .map_err(EnvironmentError::Recoverable)?;
                    // Reading the pointer avoids opening a managed environment,
                    // which may need git or network access.
                    matches!(
                        DotFlox::open_in(&path)?.pointer,
                        EnvironmentPointer::Path(_)
                    )
                },
                (AutoUpgrade::IfPathEnvironment, IncludeDescriptor::Remote { .. }) => false,
            },
        )
    }

    /// Fetch the latest version of an included environment if it's upgraded
    /// automatically, see [Self::is_auto_upgraded].
    ///
    /// For an included path environment, those are its latest locked changes.
    /// Returns [None] for an included environment that only
    /// 'flox include upgrade' fetches again.
    /// Fails if the latest changes to any path environment included below it
    /// can't be read, so that the including environment keeps its own copy.
    pub fn fetch_if_auto_upgraded(
        &self,
        flox: &Flox,
        include_environment: &IncludeDescriptor,
    ) -> Result<Option<LockedInclude>, EnvironmentError> {
        if !self.is_auto_upgraded(include_environment)? {
            return Ok(None);
        }
        self.fetch_with(flox, include_environment, UnreadableIncludes::Fail)
            .map(|fetched| Some(fetched.locked_include))
    }

    fn fetch_with(
        &self,
        flox: &Flox,
        include_environment: &IncludeDescriptor,
        unreadable_includes: UnreadableIncludes,
    ) -> Result<FetchedInclude, EnvironmentError> {
        let (manifest, lockfile, name, generation) = match include_environment {
            IncludeDescriptor::Local { dir, name, .. } => self
                .fetch_local(flox, dir, name, unreadable_includes)
                .map(|(manifest, lockfile, name)| (manifest, lockfile, name, None)),
            IncludeDescriptor::Remote {
                remote,
                name,
                generation,
                ..
            } => self
                .fetch_remote(flox, remote, name, *generation)
                // One read for both, so the manifest matches the packages
                // even if a remote environment's live generation moves.
                .map(|(lockfile, generation, name)| {
                    (lockfile.manifest.clone(), lockfile, name, Some(generation))
                }),
        }?;
        // The generation of an environment included from FloxHub already
        // identifies what it locks and includes
        let (packages_hash, included_remotes) = match include_environment {
            IncludeDescriptor::Local { dir, .. } => (
                Some(lockfile.packages_hash()),
                self.remotes_included_by(dir, &lockfile),
            ),
            IncludeDescriptor::Remote { .. } => (None, Vec::new()),
        };

        Ok(FetchedInclude {
            locked_include: LockedInclude {
                manifest,
                name,
                descriptor: include_environment.clone(),
                generation,
                packages_hash,
                included_remotes,
            },
            lockfile,
        })
    }

    /// Fetch a local (path or managed) environment, only if it's locked.
    ///
    /// A path environment provides the manifest in its lockfile,
    /// merged with the latest changes to its own automatically upgraded
    /// includes.
    /// A managed environment has to be in sync with its current generation.
    fn fetch_local(
        &self,
        flox: &Flox,
        dir: impl AsRef<Path>,
        name: &Option<String>,
        unreadable_includes: UnreadableIncludes,
    ) -> Result<(Manifest<TypedOnly>, Lockfile, String), EnvironmentError> {
        if self.base_directory.is_none() {
            return Err(EnvironmentError::Recoverable(
                RecoverableMergeError::RemoteCannotIncludeLocal,
            ));
        };

        let path = self
            .expand_include_dir(dir)
            .map_err(EnvironmentError::Recoverable)?;
        let dot_flox = DotFlox::open_in(&path)?;
        if let Some(start) = self
            .composers
            .iter()
            .position(|composer| **composer == dot_flox.path)
        {
            let cycle = self.composers[start..]
                .iter()
                .map(|composer| composer.to_path_buf())
                .chain([dot_flox.path])
                .collect();
            return Err(EnvironmentError::Recoverable(
                RecoverableMergeError::IncludeCycle(cycle),
            ));
        }

        let key = (dot_flox.path.clone(), unreadable_includes);
        if let Some(fetched) = self.fetched.local(&key, &self.composers) {
            let name = name.clone().unwrap_or(fetched.name);
            return Ok((fetched.manifest, fetched.lockfile, name));
        }
        let start = self.fetched.start_local(&dot_flox.path);

        if let EnvironmentPointer::Managed(pointer) = &dot_flox.pointer {
            let (manifest, lockfile) =
                self.fetch_managed_dir(flox, &path, &dot_flox, pointer, unreadable_includes)?;
            let environment_name = pointer.name.to_string();
            self.fetched.insert_local(
                key,
                start,
                environment_name.clone(),
                manifest.clone(),
                lockfile.clone(),
            );
            let name = name.clone().unwrap_or(environment_name);
            return Ok((manifest, lockfile, name));
        }

        let environment =
            UninitializedEnvironment::DotFlox(dot_flox).into_concrete_environment(flox, None)?;
        let environment_name = environment.name().to_string();

        let (manifest, lockfile) = match environment {
            ConcreteEnvironment::Path(environment) => {
                let include_fetcher =
                    self.for_included(environment.parent_path()?, environment.dot_flox_path());
                let core_environment =
                    environment.into_core_environment_with_include_fetcher(include_fetcher);
                // Only changes that the included environment has locked are
                // used, since locking validates them.
                let Some(lockfile) = core_environment.lockfile_if_up_to_date()? else {
                    return Err(EnvironmentError::Recoverable(
                        RecoverableMergeError::PathOutOfSync(path),
                    ));
                };
                let (manifest, seed) = core_environment.manifest_following_includes(
                    flox,
                    lockfile,
                    unreadable_includes,
                )?;
                (manifest, seed)
            },
            ConcreteEnvironment::Managed(_) | ConcreteEnvironment::Remote(_) => {
                unreachable!("a path pointer opens a path environment");
            },
        };

        self.fetched.insert_local(
            key,
            start,
            environment_name.clone(),
            manifest.clone(),
            lockfile.clone(),
        );
        let name = name.clone().unwrap_or(environment_name);
        Ok((manifest, lockfile, name))
    }

    /// Read the current generation of an environment pulled from FloxHub into
    /// the included directory `path`, as this machine has it, with the latest
    /// changes to the environments it follows, see
    /// [CoreEnvironment::manifest_following_includes].
    ///
    /// Unlike opening it, this doesn't wait for the floxmeta lock, contact
    /// FloxHub or write anything.
    /// Like a path environment's unlocked changes, changes in the directory
    /// that aren't in a generation aren't used.
    fn fetch_managed_dir(
        &self,
        flox: &Flox,
        path: &Path,
        dot_flox: &DotFlox,
        pointer: &ManagedPointer,
        unreadable_includes: UnreadableIncludes,
    ) -> Result<(Manifest<TypedOnly>, Lockfile), EnvironmentError> {
        let not_fetched = || {
            EnvironmentError::Recoverable(RecoverableMergeError::ManagedNotFetched(
                path.to_path_buf(),
            ))
        };
        let generations = local_generations(flox, pointer, &dot_flox.path)
            .map_err(ManagedEnvironmentError::FloxmetaBranch)?
            .ok_or_else(not_fetched)?;
        let (Ok(lockfile_contents), Ok(manifest_contents)) = (
            generations.current_gen_lockfile(),
            generations.current_gen_manifest_contents(),
        ) else {
            return Err(not_fetched());
        };
        let lockfile =
            Lockfile::from_str(&lockfile_contents).map_err(EnvironmentError::Lockfile)?;

        let dot_flox_path = CanonicalPath::new(&dot_flox.path)
            .map_err(|err| EnvironmentError::DotFloxNotFound(err.path))?;
        let include_fetcher = self.for_included(path.to_path_buf(), dot_flox_path);
        let env_dir = dot_flox.path.join(ENV_DIR_NAME);
        // The directory only has a copy of the generation once it's been used
        let generation_copy;
        let env_dir = if env_dir.exists() {
            let checkout = CoreEnvironment::new(&env_dir, include_fetcher.clone());
            if !ManagedEnvironment::validate_checkout(&checkout, &generations)? {
                return Err(EnvironmentError::Recoverable(
                    RecoverableMergeError::ManagedOutOfSync(path.to_path_buf()),
                ));
            }
            env_dir
        } else {
            generation_copy = tempfile::tempdir_in(&flox.temp_dir)
                .map_err(CoreEnvironmentError::MakeTemporaryEnv)?;
            fs::write(
                generation_copy.path().join(MANIFEST_FILENAME),
                manifest_contents,
            )
            .map_err(CoreEnvironmentError::MakeTemporaryEnv)?;
            fs::write(
                generation_copy.path().join(LOCKFILE_FILENAME),
                lockfile_contents,
            )
            .map_err(CoreEnvironmentError::MakeTemporaryEnv)?;
            generation_copy.path().to_path_buf()
        };
        // Like a path environment, it follows its own includes
        CoreEnvironment::new(env_dir, include_fetcher).manifest_following_includes(
            flox,
            lockfile,
            unreadable_includes,
        )
    }

    /// Fetch a remote environment.
    /// If `generation` is not [None], retrieve the lockfile for the named generation,
    /// instead of the "live" generation.
    fn fetch_remote(
        &self,
        flox: &Flox,
        remote: &RemoteEnvironmentRef,
        name: &Option<String>,
        generation: Option<usize>,
    ) -> Result<(Lockfile, usize, String), EnvironmentError> {
        let name = name.clone().unwrap_or_else(|| remote.name().to_string());
        let key = (remote.clone(), generation, self.remote_source);
        if let Some((lockfile, generation)) = self.fetched.remote(&key) {
            return Ok((lockfile, generation, name));
        }

        let pointer =
            ManagedPointer::new(remote.owner().clone(), remote.name().clone(), &flox.floxhub);
        let generations = match self.remote_source {
            RemoteSource::FloxHub => fetch_remote_generations(flox, &pointer)
                .map_err(ManagedEnvironmentError::FloxmetaBranch)?,
            RemoteSource::LastFetched => last_fetched_remote_generations(flox, &pointer)
                .map_err(ManagedEnvironmentError::FloxmetaBranch)?
                .ok_or_else(|| {
                    EnvironmentError::Recoverable(RecoverableMergeError::RemoteNotFetched(
                        remote.clone(),
                    ))
                })?,
        };
        let fetched = match generation {
            Some(generation) => generations
                .lockfile(generation)
                .map(|lockfile| (lockfile, generation)),
            None => generations.metadata().and_then(|metadata| {
                let current_gen = *metadata
                    .current_gen()
                    .ok_or(GenerationsError::NoGenerations)?;
                let lockfile = generations.lockfile_unchecked(current_gen)?;
                Ok((lockfile, current_gen))
            }),
        }
        .map_err(ManagedEnvironmentError::Generations)?;

        self.fetched.insert_remote(key, fetched.clone());
        let (lockfile, generation) = fetched;
        Ok((lockfile, generation, name))
    }

    /// The environments included from FloxHub without a pinned generation by
    /// `lockfile`, and by the path environments it includes, recursively, as
    /// their lockfiles record them
    pub(crate) fn remote_includes(&self, lockfile: &Lockfile) -> HashSet<RemoteEnvironmentRef> {
        let mut remotes = HashSet::new();
        self.collect_remote_includes(lockfile, &mut HashSet::new(), &mut remotes);
        remotes
    }

    fn collect_remote_includes(
        &self,
        lockfile: &Lockfile,
        visited: &mut HashSet<PathBuf>,
        remotes: &mut HashSet<RemoteEnvironmentRef>,
    ) {
        let Some(compose) = &lockfile.compose else {
            return;
        };
        for locked in &compose.include {
            match &locked.descriptor {
                IncludeDescriptor::Remote {
                    remote,
                    generation: None,
                    ..
                } => {
                    remotes.insert(remote.clone());
                },
                IncludeDescriptor::Remote { .. } => {},
                IncludeDescriptor::Local { dir, .. } => {
                    let Ok(path) = self.expand_include_dir(dir) else {
                        continue;
                    };
                    let Ok(dot_flox) = DotFlox::open_in(&path) else {
                        continue;
                    };
                    // An environment pulled from FloxHub follows its includes too
                    if !visited.insert(dot_flox.path.clone()) {
                        continue;
                    }
                    let lockfile_path = dot_flox.path.join(ENV_DIR_NAME).join(LOCKFILE_FILENAME);
                    let Some(lockfile) = CanonicalPath::new(lockfile_path)
                        .ok()
                        .and_then(|path| Lockfile::read_from_file(&path).ok())
                    else {
                        continue;
                    };
                    IncludeFetcher::new(Some(path))
                        .collect_remote_includes(&lockfile, visited, remotes);
                },
            }
        }
    }

    /// Whether the generations of an environment on FloxHub as last fetched
    /// don't include `generation`, because they were fetched before it was
    /// created, or haven't been fetched on this machine at all
    pub(crate) fn last_fetched_lacks_generation(
        &self,
        flox: &Flox,
        remote: &RemoteEnvironmentRef,
        generation: usize,
    ) -> bool {
        let pointer =
            ManagedPointer::new(remote.owner().clone(), remote.name().clone(), &flox.floxhub);
        let Ok(Some(generations)) = last_fetched_remote_generations(flox, &pointer) else {
            return true;
        };
        generations.metadata().map_or(true, |metadata| {
            !metadata.generations().contains_key(&generation.into())
        })
    }

    /// The environments included from FloxHub that `lockfile`, the lockfile
    /// in use, merges: directly, and through the directories it includes,
    /// recursively, if `following`, i.e. `lockfile` is the copy of an
    /// environment that follows its includes.
    ///
    /// Each is listed with the versions that `lockfile` merges, noting those
    /// that can change without `lockfile` changing: followed ones, and all of
    /// those included by a followed directory.
    /// A followed directory whose latest version is in use is read as
    /// following merges it, to get the versions of the environments it
    /// follows.
    /// Otherwise `lockfile` records what an included directory includes,
    /// unless an older version of Flox wrote it, in which case that's read
    /// from the directory's lockfile.
    pub(crate) fn included_remote_environments(
        &self,
        flox: &Flox,
        lockfile: &Lockfile,
        following: bool,
    ) -> Vec<IncludedRemoteEnvironment> {
        let includes = lockfile
            .compose
            .iter()
            .flat_map(|compose| compose.include.clone())
            .collect();
        let mut remotes = Vec::new();
        self.reading_last_fetched_remotes()
            .collect_included_remote_environments(
                flox,
                includes,
                following,
                false,
                &mut HashSet::new(),
                &mut remotes,
            );
        remotes
    }

    /// Whether `locked`, an included directory in a lockfile in use, is the
    /// directory's latest version, so the version in use is the one
    /// following computes
    fn is_latest_version(&self, flox: &Flox, locked: &LockedInclude) -> bool {
        matches!(
            self.fetch_if_auto_upgraded(flox, &locked.descriptor),
            Ok(Some(latest)) if latest.is_recorded_by(locked)
        )
    }

    /// Add the environments included from FloxHub by `includes`, which are in
    /// use, to `remotes`, see [Self::included_remote_environments].
    ///
    /// `following` is whether the environment whose includes these are
    /// follows them, and `in_followed` whether that environment is itself
    /// included by a followed directory.
    fn collect_included_remote_environments(
        &self,
        flox: &Flox,
        includes: Vec<LockedInclude>,
        following: bool,
        in_followed: bool,
        visited: &mut HashSet<(PathBuf, bool)>,
        remotes: &mut Vec<IncludedRemoteEnvironment>,
    ) {
        for locked in includes {
            let auto_upgraded =
                following && self.is_auto_upgraded(&locked.descriptor).unwrap_or(false);
            let followed = in_followed || auto_upgraded;
            let dir = match &locked.descriptor {
                IncludeDescriptor::Remote { remote, .. } => {
                    list_remote(remotes, remote, Some(locked.manifest), followed);
                    continue;
                },
                IncludeDescriptor::Local { dir, .. } => dir,
            };
            if auto_upgraded && self.is_latest_version(flox, &locked) {
                let Some((fetcher, env_dir, lockfile)) = self.included_lockfile(dir, true, visited)
                else {
                    continue;
                };
                let nested = CoreEnvironment::new(env_dir, fetcher.clone())
                    .check_auto_upgraded_includes(flox, &lockfile)
                    .includes;
                fetcher.collect_included_remote_environments(
                    flox, nested, true, true, visited, remotes,
                );
                continue;
            }
            if locked.packages_hash.is_some() {
                for included in &locked.included_remotes {
                    let manifest = included.generation.and_then(|generation| {
                        self.fetch_remote(flox, &included.remote, &None, Some(generation))
                            .ok()
                            .map(|(lockfile, ..)| lockfile.manifest)
                    });
                    // Without that generation on this machine, the manifest
                    // that merges it is shown instead,
                    // and it can't be checked for changes
                    let followed = followed && manifest.is_some();
                    let manifest = manifest.unwrap_or_else(|| locked.manifest.clone());
                    list_remote(remotes, &included.remote, Some(manifest), followed);
                }
                continue;
            }
            // As far as it can be told without a record, the version in use
            // includes what the directory's lockfile does
            let Some((fetcher, _, lockfile)) = self.included_lockfile(dir, false, visited) else {
                continue;
            };
            let nested = lockfile
                .compose
                .into_iter()
                .flat_map(|compose| compose.include)
                .collect();
            fetcher
                .collect_included_remote_environments(flox, nested, false, false, visited, remotes);
        }
    }

    /// The environments from FloxHub that `lockfile`, the lockfile in use of
    /// the environment in the included directory `dir`, merges: directly, and
    /// through the directories that environment includes, as `lockfile`
    /// records them.
    ///
    /// For an included directory that it doesn't record them for, because
    /// an older version of Flox wrote it, they're read from that directory's
    /// lockfile.
    fn remotes_included_by(&self, dir: &Path, lockfile: &Lockfile) -> Vec<IncludedRemote> {
        let mut visited = HashSet::new();
        let Some((fetcher, _, _)) = self.included_lockfile(dir, false, &mut visited) else {
            return Vec::new();
        };
        let mut remotes = Vec::new();
        fetcher.collect_remotes_included_by(lockfile, &mut visited, &mut remotes);
        remotes
    }

    fn collect_remotes_included_by(
        &self,
        lockfile: &Lockfile,
        visited: &mut HashSet<(PathBuf, bool)>,
        remotes: &mut Vec<IncludedRemote>,
    ) {
        fn add(remotes: &mut Vec<IncludedRemote>, remote: IncludedRemote) {
            if !remotes.contains(&remote) {
                remotes.push(remote);
            }
        }
        for locked in lockfile.compose.iter().flat_map(|compose| &compose.include) {
            match &locked.descriptor {
                IncludeDescriptor::Remote { remote, .. } => add(remotes, IncludedRemote {
                    remote: remote.clone(),
                    generation: locked.generation,
                }),
                IncludeDescriptor::Local { .. } if locked.packages_hash.is_some() => {
                    for remote in &locked.included_remotes {
                        add(remotes, remote.clone());
                    }
                },
                IncludeDescriptor::Local { dir, .. } => {
                    if let Some((fetcher, _, lockfile)) =
                        self.included_lockfile(dir, false, visited)
                    {
                        fetcher.collect_remotes_included_by(&lockfile, visited, remotes);
                    }
                },
            }
        }
    }

    /// A fetcher for the environment in an included directory, its
    /// environment directory and its lockfile, unless it can't be read or was
    /// already visited, as followed or not
    fn included_lockfile(
        &self,
        dir: &Path,
        followed: bool,
        visited: &mut HashSet<(PathBuf, bool)>,
    ) -> Option<(Self, PathBuf, Lockfile)> {
        let path = self.expand_include_dir(dir).ok()?;
        let dot_flox = DotFlox::open_in(&path).ok()?;
        if !visited.insert((dot_flox.path.clone(), followed)) {
            return None;
        }
        let env_dir = dot_flox.path.join(ENV_DIR_NAME);
        let lockfile =
            Lockfile::read_from_file(&CanonicalPath::new(env_dir.join(LOCKFILE_FILENAME)).ok()?)
                .ok()?;
        let fetcher = self.for_included(path, CanonicalPath::new(&dot_flox.path).ok()?);
        Some((fetcher, env_dir, lockfile))
    }

    /// The latest generation of an environment on FloxHub as last fetched,
    /// and its lockfile, without contacting FloxHub
    pub(crate) fn last_fetched_remote(
        &self,
        flox: &Flox,
        remote: &RemoteEnvironmentRef,
    ) -> Result<(Lockfile, usize), EnvironmentError> {
        self.reading_last_fetched_remotes()
            .fetch_remote(flox, remote, &None, None)
            .map(|(lockfile, generation, _)| (lockfile, generation))
    }

    /// For directories that aren't absolute, join them to the base_directory
    /// for this IncludeFetcher
    pub fn expand_include_dir(
        &self,
        dir: impl AsRef<Path>,
    ) -> Result<PathBuf, RecoverableMergeError> {
        let Some(base_directory) = &self.base_directory else {
            return Err(RecoverableMergeError::RemoteCannotIncludeLocal);
        };

        let dir = dir.as_ref();

        Ok(if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            base_directory.join(dir)
        })
    }
}

pub mod test_helpers {
    use super::*;

    /// Returns an IncludeFetcher that fails to fetch anything
    pub fn mock_include_fetcher() -> IncludeFetcher {
        IncludeFetcher::new(None)
    }
}

/// Add `manifest`, a version in use of the environment from FloxHub
/// `env_ref`, to `remotes`, listing each environment once
fn list_remote(
    remotes: &mut Vec<IncludedRemoteEnvironment>,
    env_ref: &RemoteEnvironmentRef,
    manifest: Option<Manifest<TypedOnly>>,
    followed: bool,
) {
    let listed = match remotes.iter().position(|listed| &listed.env_ref == env_ref) {
        Some(index) => &mut remotes[index],
        None => {
            remotes.push(IncludedRemoteEnvironment {
                env_ref: env_ref.clone(),
                manifests: Vec::new(),
                followed: Vec::new(),
            });
            remotes.last_mut().expect("just pushed")
        },
    };
    let Some(manifest) = manifest else {
        return;
    };
    if followed && !listed.followed.contains(&manifest) {
        listed.followed.push(manifest.clone());
    }
    if !listed.manifests.contains(&manifest) {
        listed.manifests.push(manifest);
    }
}

#[cfg(test)]
mod test {
    use std::fs;

    use flox_core::data::environment_ref::EnvironmentOwner;
    use flox_manifest::interfaces::AsTypedOnlyManifest;
    use flox_manifest::test_helpers::with_latest_schema;
    use indoc::{formatdoc, indoc};
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use crate::models::env_registry::{env_registry_path, read_environment_registry};
    use crate::models::environment::floxmeta_branch::GenerationLock;
    use crate::models::environment::generations::GenerationsExt;
    use crate::models::environment::managed_environment::GENERATION_LOCK_FILENAME;
    use crate::models::environment::managed_environment::test_helpers::mock_managed_environment_in;
    use crate::models::environment::path_environment::test_helpers::new_path_environment_in;
    use crate::models::environment::remote_environment::RemoteEnvironment;
    use crate::models::environment::remote_environment::test_helpers::mock_remote_environment;
    use crate::models::floxmeta::floxmeta_dir;
    use crate::providers::git::{GitCommandProvider, GitProvider};
    use crate::providers::lock_manifest::LockResult;

    #[test]
    fn fetch_path_relative_path() {
        let (flox, tempdir) = flox_instance();

        let environment_path = tempdir.path().join("environment");
        let manifest_contents = with_latest_schema("");
        let manifest = toml_edit::de::from_str(&manifest_contents).unwrap();

        fs::create_dir(&environment_path).unwrap();
        let mut environment = new_path_environment_in(&flox, &manifest_contents, &environment_path);
        let lockfile: Lockfile = environment.lockfile(&flox).unwrap().into();

        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));

        let include_descriptor = IncludeDescriptor::Local {
            dir: environment_path.file_name().unwrap().into(),
            name: None,
            auto_upgrade: None,
        };

        let fetched = include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        assert_eq!(fetched, FetchedInclude {
            locked_include: LockedInclude {
                manifest,
                name: "environment".to_string(),
                descriptor: include_descriptor,
                generation: None,
                packages_hash: Some(lockfile.packages_hash()),
                included_remotes: Vec::new(),
            },
            lockfile,
        })
    }

    #[test]
    fn fetch_path_absolute_path() {
        let (flox, tempdir) = flox_instance();

        let environment_path = tempdir.path().join("environment");
        let manifest_contents = with_latest_schema("");
        let manifest = toml_edit::de::from_str(&manifest_contents).unwrap();

        fs::create_dir(&environment_path).unwrap();
        let mut environment = new_path_environment_in(&flox, &manifest_contents, &environment_path);
        let lockfile: Lockfile = environment.lockfile(&flox).unwrap().into();

        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));

        let include_descriptor = IncludeDescriptor::Local {
            dir: environment_path,
            name: None,
            auto_upgrade: None,
        };

        let fetched = include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        assert_eq!(fetched, FetchedInclude {
            locked_include: LockedInclude {
                manifest,
                name: "environment".to_string(),
                descriptor: include_descriptor,
                generation: None,
                packages_hash: Some(lockfile.packages_hash()),
                included_remotes: Vec::new(),
            },
            lockfile,
        })
    }

    /// For fetching path environments:
    /// - Fetching fails when not locked
    /// - Fetching succeeds for trivial changes in the manifest (e.g. comments)
    /// - Fetching fails when there are non-trivial changes in the manifest not
    ///   reflected in the lockfile
    #[test]
    fn fetch_path_fails_if_out_of_sync() {
        let (flox, tempdir) = flox_instance();

        let environment_path = tempdir.path().join("environment");
        let manifest_contents = with_latest_schema("");

        fs::create_dir(&environment_path).unwrap();
        let mut environment = new_path_environment_in(&flox, &manifest_contents, &environment_path);

        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));

        let include_descriptor = IncludeDescriptor::Local {
            dir: environment_path.file_name().unwrap().into(),
            name: None,
            auto_upgrade: None,
        };

        let expected_error = formatdoc! {r#"
        cannot include environment since its manifest and lockfile are out of sync

        To resolve this issue run 'flox edit -d {}' and retry
        "#, environment_path.to_string_lossy()};

        // Fetching should fail before locking
        let err = include_fetcher
            .fetch(&flox, &include_descriptor)
            .unwrap_err();
        assert_eq!(err.to_string(), expected_error);

        // After locking, fetching should succeed
        environment.lockfile(&flox).unwrap();
        include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        // After writing a comment, fetching should succeed
        fs::write(
            environment.manifest_path(&flox).unwrap(),
            with_latest_schema("# comment"),
        )
        .unwrap();
        include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        // After writing an actual change, fetching should fail
        fs::write(
            environment.manifest_path(&flox).unwrap(),
            with_latest_schema(indoc! {r#"
                # comment
                [vars]
                foo = "bar"
            "#}),
        )
        .unwrap();
        // A new command fetches it again
        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));
        let err = include_fetcher
            .fetch(&flox, &include_descriptor)
            .unwrap_err();
        assert_eq!(err.to_string(), expected_error);
    }

    /// Fetching an environment that is already being composed is a cycle
    #[test]
    fn fetch_path_errors_on_include_cycle() {
        let (flox, tempdir) = flox_instance();

        let a_path = tempdir.path().join("a");
        let b_path = tempdir.path().join("b");
        fs::create_dir(&a_path).unwrap();
        fs::create_dir(&b_path).unwrap();
        // b can only include a while a doesn't include b yet
        let mut a = new_path_environment_in(&flox, &with_latest_schema(""), &a_path);
        a.lockfile(&flox).unwrap();
        let mut b = new_path_environment_in(
            &flox,
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../a\" }]"),
            &b_path,
        );
        b.lockfile(&flox).unwrap();
        fs::write(
            a.manifest_path(&flox).unwrap(),
            with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        )
        .unwrap();

        let include_fetcher = IncludeFetcher::for_composer(a_path.clone(), a.dot_flox_path());

        let self_include = IncludeDescriptor::Local {
            dir: ".".into(),
            name: None,
            auto_upgrade: None,
        };
        let err = include_fetcher.fetch(&flox, &self_include).unwrap_err();
        let EnvironmentError::Recoverable(RecoverableMergeError::IncludeCycle(cycle)) = err else {
            panic!("expected an include cycle, got: {err:?}");
        };
        assert_eq!(cycle, vec![
            a.dot_flox_path().to_path_buf(),
            a.dot_flox_path().to_path_buf()
        ]);

        let include_b = IncludeDescriptor::Local {
            dir: "../b".into(),
            name: None,
            auto_upgrade: None,
        };
        let err = include_fetcher.fetch(&flox, &include_b).unwrap_err();
        let EnvironmentError::Recoverable(RecoverableMergeError::IncludeCycle(cycle)) = err else {
            panic!("expected an include cycle, got: {err:?}");
        };
        assert_eq!(cycle, vec![
            a.dot_flox_path().to_path_buf(),
            b.dot_flox_path().to_path_buf(),
            a.dot_flox_path().to_path_buf()
        ]);
    }

    /// A managed environment in an included directory isn't upgraded
    /// automatically by default, so checking it for changes doesn't open it
    #[test]
    fn fetch_if_auto_upgraded_skips_managed_directory() {
        let owner = "owner".parse().unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));

        let environment_path = tempdir.path().join("environment");
        fs::create_dir(&environment_path).unwrap();
        let environment =
            mock_managed_environment_in(&flox, "version = 1\n", owner, &environment_path, None);
        // Local changes would make fetching the managed environment fail
        fs::write(
            environment.manifest_path(&flox).unwrap(),
            "version = 1\n[vars]\nfoo = \"bar\"\n",
        )
        .unwrap();

        let fetched = IncludeFetcher::new(Some(tempdir.path().to_path_buf()))
            .fetch_if_auto_upgraded(&flox, &IncludeDescriptor::Local {
                dir: environment_path.file_name().unwrap().into(),
                name: None,
                auto_upgrade: None,
            })
            .unwrap();
        assert_eq!(fetched, None);
    }

    /// An environment pulled from FloxHub into an included directory is read
    /// without waiting for the floxmeta lock, which another command may hold
    /// while it fetches
    #[test]
    fn fetch_managed_does_not_wait_for_floxmeta_lock() {
        let owner: EnvironmentOwner = "owner".parse().unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let environment_path = tempdir.path().join("environment");
        fs::create_dir(&environment_path).unwrap();
        let manifest_contents = with_latest_schema("[vars]\nfoo = \"bar\"");
        mock_managed_environment_in(
            &flox,
            &manifest_contents,
            owner.clone(),
            &environment_path,
            None,
        );
        let floxmeta_dir = floxmeta_dir(&flox, &owner);
        let mut lock = fslock::LockFile::open(&floxmeta_dir.with_file_name(format!(
            "{}.lock",
            floxmeta_dir.file_name().unwrap().to_string_lossy()
        )))
        .unwrap();
        lock.lock().unwrap();

        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));
        let include_descriptor = IncludeDescriptor::Local {
            dir: environment_path.file_name().unwrap().into(),
            name: None,
            auto_upgrade: None,
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(include_fetcher.fetch(&flox, &include_descriptor));
        });
        let fetched = receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("fetching waited for the floxmeta lock")
            .unwrap();
        assert_eq!(
            fetched.locked_include.manifest,
            toml_edit::de::from_str(&manifest_contents).unwrap()
        );
    }

    /// An environment pulled from FloxHub into an included directory whose
    /// current generation this machine doesn't have isn't fetched
    #[test]
    fn fetch_managed_fails_without_its_generation() {
        let owner = "owner".parse().unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let environment_path = tempdir.path().join("environment");
        fs::create_dir(&environment_path).unwrap();
        let environment =
            mock_managed_environment_in(&flox, "version = 1\n", owner, &environment_path, None);
        let lock_path = environment.dot_flox_path().join(GENERATION_LOCK_FILENAME);
        let mut lock = GenerationLock::read_maybe(&lock_path).unwrap().unwrap();
        lock.rev = "0".repeat(40);
        lock.local_rev = None;
        fs::write(&lock_path, serde_json::to_string(&lock).unwrap()).unwrap();

        let err = IncludeFetcher::new(Some(tempdir.path().to_path_buf()))
            .fetch(&flox, &IncludeDescriptor::Local {
                dir: environment_path.file_name().unwrap().into(),
                name: None,
                auto_upgrade: None,
            })
            .unwrap_err();
        assert!(
            matches!(
                err,
                EnvironmentError::Recoverable(RecoverableMergeError::ManagedNotFetched(_))
            ),
            "{err:?}"
        );
    }

    /// fetch() errors if attempting to fetch an out of sync managed environment
    #[test]
    fn fetch_managed_fails_if_out_of_sync() {
        let owner = "owner".parse().unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));

        let environment_path = tempdir.path().join("environment");
        let manifest_contents = indoc! {r#"
        version = 1
        "#};

        fs::create_dir(&environment_path).unwrap();
        let environment =
            mock_managed_environment_in(&flox, manifest_contents, owner, &environment_path, None);

        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));

        let include_descriptor = IncludeDescriptor::Local {
            dir: environment_path.file_name().unwrap().into(),
            name: None,
            auto_upgrade: None,
        };

        // After writing a comment, fetching should fail
        fs::write(environment.manifest_path(&flox).unwrap(), indoc! {r#"
        version = 1

        # comment
        "#})
        .unwrap();
        let err = include_fetcher
            .fetch(&flox, &include_descriptor)
            .unwrap_err();
        assert!(err.to_string().contains(
            "cannot include environment since it has changes not yet synced to a generation"
        ));
    }

    #[test]
    fn fetch_remote() {
        let env_ref = RemoteEnvironmentRef::new("owner", "name").unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(env_ref.owner()));

        let mut remote_env = mock_remote_environment(
            &flox,
            &with_latest_schema(""),
            env_ref.owner().clone(),
            Some(&env_ref.name().to_string()),
        );

        // Open the remote environment in the default location to simulate an existing activation.
        let mut open_env = RemoteEnvironment::new(
            &flox,
            ManagedPointer::new(
                env_ref.owner().clone(),
                env_ref.name().clone(),
                &flox.floxhub,
            ),
            None,
        )
        .unwrap();
        let open_env_lockfile_previous = match open_env.lockfile(&flox).unwrap() {
            LockResult::Unchanged(lockfile) => lockfile,
            LockResult::Changed(_) => {
                panic!("remote environments should already be locked")
            },
        };

        // Modify the remote environment with a new generation.
        let manifest_contents = with_latest_schema(indoc! {r#"
            [vars]
            foo = "bar"
        "#});
        let manifest = toml_edit::de::from_str(&manifest_contents).unwrap();
        remote_env
            .edit(&flox, manifest_contents.to_string())
            .unwrap();
        remote_env.push(&flox, true).unwrap();
        let lockfile = remote_env.existing_lockfile(&flox).unwrap().unwrap();
        let current_generation = *remote_env
            .generations_metadata()
            .unwrap()
            .current_gen()
            .unwrap();

        // Fetch and lock the remote environment.
        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));
        let include_descriptor = IncludeDescriptor::Remote {
            remote: "owner/name".parse().unwrap(),
            name: None,
            generation: None,
            auto_upgrade: None,
        };
        let fetched = include_fetcher.fetch(&flox, &include_descriptor).unwrap();
        assert_eq!(
            fetched,
            FetchedInclude {
                locked_include: LockedInclude {
                    manifest,
                    name: "name".to_string(),
                    descriptor: include_descriptor,
                    generation: Some(current_generation),
                    packages_hash: None,
                    included_remotes: Vec::new(),
                },
                lockfile,
            },
            "fetch should get the new generation"
        );

        let open_env_lockfile_now = match open_env.lockfile(&flox).unwrap() {
            LockResult::Unchanged(lockfile) => lockfile,
            LockResult::Changed(_) => {
                panic!("remote environments should already be locked")
            },
        };
        assert_eq!(
            open_env_lockfile_now, open_env_lockfile_previous,
            "fetch should not affect the generation of an already open environment"
        );
    }

    /// Fetching a remote environment doesn't register an environment or
    /// create a floxmeta branch that only garbage collection would remove
    #[test]
    fn fetch_remote_leaves_nothing_behind() {
        let env_ref = RemoteEnvironmentRef::new("owner", "name").unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(env_ref.owner()));
        mock_remote_environment(
            &flox,
            &with_latest_schema(""),
            env_ref.owner().clone(),
            Some(&env_ref.name().to_string()),
        );
        let registry = || read_environment_registry(env_registry_path(&flox)).unwrap();
        let branches = || {
            GitCommandProvider::open(floxmeta_dir(&flox, env_ref.owner()))
                .unwrap()
                .list_branches()
                .unwrap()
                .into_iter()
                .map(|branch| branch.name)
                .collect::<Vec<_>>()
        };
        let registry_before = registry();
        let branches_before = branches();

        IncludeFetcher::new(Some(tempdir.path().to_path_buf()))
            .fetch(&flox, &IncludeDescriptor::Remote {
                remote: env_ref.clone(),
                name: None,
                generation: None,
                auto_upgrade: None,
            })
            .unwrap();

        assert_eq!(registry(), registry_before);
        assert_eq!(branches(), branches_before);
    }

    #[test]
    fn fetch_remote_with_generation() {
        let env_ref = RemoteEnvironmentRef::new("owner", "name").unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(env_ref.owner()));

        let mut remote_env = mock_remote_environment(
            &flox,
            &with_latest_schema(""),
            env_ref.owner().clone(),
            Some(&env_ref.name().to_string()),
        );

        let initial_generation = remote_env
            .generations_metadata()
            .unwrap()
            .current_gen()
            .unwrap();
        let initial_generation_manifest = remote_env.manifest(&flox).unwrap();
        let initial_generation_lockfile = remote_env.existing_lockfile(&flox).unwrap().unwrap();

        // Fetch and lock the remote environment at a given generation.
        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));
        let include_descriptor = IncludeDescriptor::Remote {
            remote: "owner/name".parse().unwrap(),
            name: None,
            generation: Some(*initial_generation),
            auto_upgrade: None,
        };

        let fetched = include_fetcher.fetch(&flox, &include_descriptor).unwrap();
        assert_eq!(fetched, FetchedInclude {
            locked_include: LockedInclude {
                manifest: initial_generation_manifest.as_typed_only(),
                name: "name".to_string(),
                descriptor: include_descriptor.clone(),
                generation: Some(*initial_generation),
                packages_hash: None,
                included_remotes: Vec::new(),
            },
            lockfile: initial_generation_lockfile,
        });

        // Modify the remote environment to create a new generation.
        let manifest_contents = with_latest_schema(indoc! {r#"
            [vars]
            foo = "bar"
        "#});
        remote_env.edit(&flox, manifest_contents.clone()).unwrap();

        let fetched_after_upstream_changes =
            include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        // include should remain at the pinned generation
        assert_eq!(
            fetched_after_upstream_changes, fetched,
            "fetch should get the locked generation"
        );
    }

    /// Only a remote environment with `auto-upgrade = true` and no pinned
    /// generation is upgraded automatically
    #[test]
    fn fetch_if_auto_upgraded_remote() {
        let env_ref = RemoteEnvironmentRef::new("owner", "name").unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(env_ref.owner()));
        let mut remote_env = mock_remote_environment(
            &flox,
            &with_latest_schema(""),
            env_ref.owner().clone(),
            Some(&env_ref.name().to_string()),
        );
        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));
        let remote = |generation, auto_upgrade| IncludeDescriptor::Remote {
            remote: env_ref.clone(),
            name: None,
            generation,
            auto_upgrade,
        };

        let not_auto_upgraded = [
            remote(None, None),
            remote(None, Some(false)),
            remote(Some(1), None),
            remote(Some(1), Some(true)),
        ]
        .map(|descriptor| {
            include_fetcher
                .fetch_if_auto_upgraded(&flox, &descriptor)
                .unwrap()
        });
        assert_eq!(not_auto_upgraded, [None, None, None, None]);

        let auto_upgraded = remote(None, Some(true));
        let fetched = include_fetcher
            .fetch_if_auto_upgraded(&flox, &auto_upgraded)
            .unwrap();
        assert_eq!(
            fetched,
            Some(LockedInclude {
                manifest: remote_env.manifest(&flox).unwrap().as_typed_only(),
                name: "name".to_string(),
                descriptor: auto_upgraded,
                generation: Some(
                    *remote_env
                        .generations_metadata()
                        .unwrap()
                        .current_gen()
                        .unwrap()
                ),
                packages_hash: None,
                included_remotes: Vec::new(),
            })
        );
    }

    /// Fetchers that share fetched includes fetch each remote environment
    /// once, so they all use the same version of it
    #[test]
    fn fetch_remote_once_with_shared_fetched_includes() {
        let env_ref = RemoteEnvironmentRef::new("owner", "name").unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(env_ref.owner()));
        let mut remote_env = mock_remote_environment(
            &flox,
            &with_latest_schema(""),
            env_ref.owner().clone(),
            Some(&env_ref.name().to_string()),
        );
        let include_descriptor = IncludeDescriptor::Remote {
            remote: env_ref.clone(),
            name: None,
            generation: None,
            auto_upgrade: None,
        };
        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));
        let fetched = include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        remote_env
            .edit(&flox, with_latest_schema("[vars]\nfoo = \"bar\"\n"))
            .unwrap();
        remote_env.push(&flox, true).unwrap();

        let sharing_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()))
            .with_fetched_includes(include_fetcher.fetched.clone());
        assert_eq!(
            sharing_fetcher.fetch(&flox, &include_descriptor).unwrap(),
            fetched
        );
        let new_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));
        assert_ne!(
            new_fetcher.fetch(&flox, &include_descriptor).unwrap(),
            fetched
        );
    }

    /// Fetchers that share fetched includes fetch each local environment
    /// once, so they all use the same version of it
    #[test]
    fn fetch_local_once_with_shared_fetched_includes() {
        let (flox, tempdir) = flox_instance();
        let mut environment = new_path_environment_in(
            &flox,
            &with_latest_schema("[vars]\nfoo = \"v1\""),
            tempdir.path().join("environment"),
        );
        environment.lockfile(&flox).unwrap();
        let include_descriptor = IncludeDescriptor::Local {
            dir: "environment".into(),
            name: None,
            auto_upgrade: None,
        };
        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));
        let fetched = include_fetcher.fetch(&flox, &include_descriptor).unwrap();

        fs::write(
            environment.manifest_path(&flox).unwrap(),
            with_latest_schema("[vars]\nfoo = \"v2\""),
        )
        .unwrap();
        environment.lockfile(&flox).unwrap();

        let sharing_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()))
            .with_fetched_includes(include_fetcher.fetched.clone());
        assert_eq!(
            sharing_fetcher.fetch(&flox, &include_descriptor).unwrap(),
            fetched
        );
        let new_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));
        assert_ne!(
            new_fetcher.fetch(&flox, &include_descriptor).unwrap(),
            fetched
        );
    }

    /// An environment fetched before isn't reused for a composer that it
    /// includes in turn, so that the include cycle is still found.
    ///
    /// D follows X, which pins D, so fetching D on its own doesn't find the
    /// cycle, but fetching it for X does.
    #[test]
    fn fetch_local_finds_cycle_through_environment_fetched_before() {
        let (flox, tempdir) = flox_instance();
        let mut x = new_path_environment_in(
            &flox,
            &with_latest_schema("[vars]\nx = \"v1\""),
            tempdir.path().join("x"),
        );
        x.lockfile(&flox).unwrap();
        let mut d = new_path_environment_in(
            &flox,
            &with_latest_schema("[vars]\nd = \"v1\""),
            tempdir.path().join("d"),
        );
        d.lockfile(&flox).unwrap();
        fs::write(
            x.manifest_path(&flox).unwrap(),
            with_latest_schema(
                "[include]\nenvironments = [{ dir = \"../d\", auto-upgrade = false }]",
            ),
        )
        .unwrap();
        x.lockfile(&flox).unwrap();
        fs::write(
            d.manifest_path(&flox).unwrap(),
            with_latest_schema("[include]\nenvironments = [{ dir = \"../x\" }]"),
        )
        .unwrap();
        d.lockfile(&flox).unwrap();

        let include_fetcher = IncludeFetcher::new(Some(tempdir.path().to_path_buf()));
        let include_d = IncludeDescriptor::Local {
            dir: "d".into(),
            name: None,
            auto_upgrade: None,
        };
        include_fetcher.fetch(&flox, &include_d).unwrap();

        let fetcher_for_x =
            IncludeFetcher::for_composer(tempdir.path().join("x"), x.dot_flox_path())
                .with_fetched_includes(include_fetcher.fetched.clone());
        let include_d_from_x = IncludeDescriptor::Local {
            dir: "../d".into(),
            name: None,
            auto_upgrade: None,
        };
        let err = fetcher_for_x.fetch(&flox, &include_d_from_x).unwrap_err();
        assert!(
            matches!(
                err,
                EnvironmentError::Recoverable(RecoverableMergeError::IncludeCycle(_))
            ),
            "{err:?}"
        );
    }
}
