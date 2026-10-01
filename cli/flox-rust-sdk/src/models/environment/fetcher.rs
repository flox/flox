use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use flox_core::data::environment_ref::RemoteEnvironmentRef;
use flox_manifest::lockfile::{LockedInclude, Lockfile};
use flox_manifest::parsed::latest::{AutoUpgrade, IncludeDescriptor};
use flox_manifest::{Manifest, TypedOnly};

use super::{
    ConcreteEnvironment,
    DotFlox,
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

/// The lockfiles of remote environments that have been fetched, by
/// environment and generation.
///
/// Sharing them between fetchers fetches each remote environment once,
/// so everything that uses it sees the same version.
pub type RemoteLockfiles = Arc<Mutex<HashMap<(RemoteEnvironmentRef, Option<usize>), Lockfile>>>;

/// Context required to fetch an environment include
#[derive(Clone, Debug)]
pub struct IncludeFetcher {
    pub base_directory: Option<PathBuf>,
    /// The `.flox` directories of the environments whose includes are being
    /// fetched, outermost first, used to detect include cycles
    composers: Vec<CanonicalPath>,
    remote_lockfiles: RemoteLockfiles,
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
            remote_lockfiles: RemoteLockfiles::default(),
        }
    }

    /// A fetcher for the includes of the environment at `dot_flox`,
    /// whose relative include directories are resolved against
    /// `base_directory`
    pub fn for_composer(base_directory: PathBuf, dot_flox: CanonicalPath) -> Self {
        Self {
            base_directory: Some(base_directory),
            composers: vec![dot_flox],
            remote_lockfiles: RemoteLockfiles::default(),
        }
    }

    /// Use, and add to, remote environments that other fetchers have fetched
    pub fn with_remote_lockfiles(mut self, remote_lockfiles: RemoteLockfiles) -> Self {
        self.remote_lockfiles = remote_lockfiles;
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
            remote_lockfiles: self.remote_lockfiles.clone(),
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
        let (manifest, lockfile, name) = match include_environment {
            IncludeDescriptor::Local { dir, name, .. } => {
                self.fetch_local(flox, dir, name, unreadable_includes)
            },
            IncludeDescriptor::Remote {
                remote,
                name,
                generation,
                ..
            } => self
                .fetch_remote(flox, remote, name, *generation)
                // One read for both, so the manifest matches the packages
                // even if a remote environment's live generation moves.
                .map(|(lockfile, name)| (lockfile.manifest.clone(), lockfile, name)),
        }?;

        Ok(FetchedInclude {
            locked_include: LockedInclude {
                manifest,
                name,
                descriptor: include_environment.clone(),
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

        let environment =
            UninitializedEnvironment::DotFlox(dot_flox).into_concrete_environment(flox, None)?;
        let name = name
            .clone()
            .unwrap_or_else(|| environment.name().to_string());

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
                (lockfile.manifest.clone(), lockfile)
            },
            ConcreteEnvironment::Remote(_) => {
                unreachable!("opening a path cannot result in a remote environment");
            },
        };

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
    ) -> Result<(Lockfile, String), EnvironmentError> {
        let name = name.clone().unwrap_or_else(|| remote.name().to_string());
        let key = (remote.clone(), generation);
        let fetched = self
            .remote_lockfiles
            .lock()
            .expect("remote lockfiles lock should not be poisoned")
            .get(&key)
            .cloned();
        if let Some(lockfile) = fetched {
            return Ok((lockfile, name));
        }

        let pointer =
            ManagedPointer::new(remote.owner().clone(), remote.name().clone(), &flox.floxhub);
        let generations = fetch_remote_generations(flox, &pointer)
            .map_err(ManagedEnvironmentError::FloxmetaBranch)?;
        let lockfile = match generation {
            Some(generation) => generations.lockfile(generation),
            None => generations.current_gen_lockfile().and_then(|contents| {
                Lockfile::from_str(&contents).map_err(GenerationsError::Lockfile)
            }),
        }
        .map_err(ManagedEnvironmentError::Generations)?;

        self.remote_lockfiles
            .lock()
            .expect("remote lockfiles lock should not be poisoned")
            .insert(key, lockfile.clone());
        Ok((lockfile, name))
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
            })
        );
    }

    /// Fetchers that share remote manifests fetch each remote environment
    /// once, so they all use the same version of it
    #[test]
    fn fetch_remote_once_with_shared_remote_lockfiles() {
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
            .with_remote_lockfiles(include_fetcher.remote_lockfiles.clone());
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
}
