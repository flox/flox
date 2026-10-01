use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use flox_core::data::environment_ref::RemoteEnvironmentRef;
use flox_manifest::lockfile::{LockedInclude, Lockfile};
use flox_manifest::parsed::latest::{AutoUpgrade, IncludeDescriptor};
use flox_manifest::{Manifest, TypedOnly};
use itertools::Itertools;

use super::core_environment::CoreEnvironment;
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
use crate::models::environment::floxmeta_branch::fetch_remote_generations;
use crate::models::environment::generations::GenerationsError;
use crate::models::environment::managed_environment::ManagedEnvironmentError;
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
    remote: HashMap<(RemoteEnvironmentRef, Option<usize>), (Lockfile, usize)>,
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

    fn remote(&self, key: &(RemoteEnvironmentRef, Option<usize>)) -> Option<(Lockfile, usize)> {
        self.lock().remote.get(key).cloned()
    }

    fn insert_remote(
        &self,
        key: (RemoteEnvironmentRef, Option<usize>),
        fetched: (Lockfile, usize),
    ) {
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

/// Context required to fetch an environment include
#[derive(Clone, Debug)]
pub struct IncludeFetcher {
    pub base_directory: Option<PathBuf>,
    /// The `.flox` directories of the environments whose includes are being
    /// fetched, outermost first, used to detect include cycles
    composers: Vec<CanonicalPath>,
    fetched: FetchedIncludes,
}

/// The included environment as fetched,
/// with the lockfile whose packages seed the composing environment's lock.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchedInclude {
    pub locked_include: LockedInclude,
    /// The included environment's lockfile.
    ///
    /// It may predate the fetched manifest, e.g. for a path environment
    /// merged with the latest changes to its own includes.
    /// Seeding only reuses the packages it locks for descriptors that the
    /// merged manifest doesn't change.
    pub lockfile: Lockfile,
}

impl IncludeFetcher {
    pub fn new(base_directory: Option<PathBuf>) -> Self {
        Self {
            base_directory,
            composers: Vec::new(),
            fetched: FetchedIncludes::default(),
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

        Ok(FetchedInclude {
            locked_include: LockedInclude {
                manifest,
                name,
                descriptor: include_environment.clone(),
                generation,
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
                let manifest = core_environment.manifest_following_includes(
                    flox,
                    lockfile.clone(),
                    unreadable_includes,
                )?;
                (manifest, lockfile)
            },
            ConcreteEnvironment::Managed(environment) => {
                let Some(lockfile) = environment.existing_lockfile(flox)? else {
                    return Err(EnvironmentError::Recoverable(
                        RecoverableMergeError::ManagedOutOfSync(path),
                    ));
                };
                if environment.has_local_changes(flox)? {
                    return Err(EnvironmentError::Recoverable(
                        RecoverableMergeError::ManagedOutOfSync(path),
                    ));
                }
                // Like a path environment, it follows its own includes
                let include_fetcher =
                    self.for_included(environment.parent_path()?, environment.dot_flox_path());
                let manifest = CoreEnvironment::new(
                    environment.dot_flox_path().join(ENV_DIR_NAME),
                    include_fetcher,
                )
                .manifest_following_includes(
                    flox,
                    lockfile.clone(),
                    unreadable_includes,
                )?;
                (manifest, lockfile)
            },
            ConcreteEnvironment::Remote(_) => {
                unreachable!("opening a path cannot result in a remote environment");
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
        let key = (remote.clone(), generation);
        if let Some((lockfile, generation)) = self.fetched.remote(&key) {
            return Ok((lockfile, generation, name));
        }

        let pointer =
            ManagedPointer::new(remote.owner().clone(), remote.name().clone(), &flox.floxhub);
        let generations = fetch_remote_generations(flox, &pointer)
            .map_err(ManagedEnvironmentError::FloxmetaBranch)?;
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

#[cfg(test)]
mod test {
    use std::fs;

    use flox_manifest::interfaces::AsTypedOnlyManifest;
    use flox_manifest::test_helpers::with_latest_schema;
    use indoc::{formatdoc, indoc};
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use crate::models::env_registry::{env_registry_path, read_environment_registry};
    use crate::models::environment::generations::GenerationsExt;
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
        let lockfile = environment.lockfile(&flox).unwrap().into();

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
        let lockfile = environment.lockfile(&flox).unwrap().into();

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
