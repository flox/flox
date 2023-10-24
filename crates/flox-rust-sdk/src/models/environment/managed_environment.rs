use std::path::{Path, PathBuf};
use std::{fs, io};

use async_trait::async_trait;
use flox_types::catalog::{EnvCatalog, System};
use flox_types::version::Version;
use log::debug;
use runix::command_line::NixCommandLine;
use runix::installable::FlakeAttribute;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{Environment, EnvironmentError2, ManagedPointer};
use crate::flox::Flox;
use crate::models::environment_ref::{EnvironmentName, EnvironmentOwner, EnvironmentRef};
use crate::models::floxmetav2::{FloxmetaV2, FloxmetaV2Error};
use crate::prelude::flox_package::FloxPackage;
use crate::providers::git::{GitCommandBranchHashError, GitCommandError};

const GENERATION_LOCK_FILENAME: &str = "env.lock";

#[derive(Debug)]
pub struct ManagedEnvironment {
    // typically `<...>/.flox`
    _path: PathBuf,
    _pointer: ManagedPointer,
    _system: String,
    _floxmeta: FloxmetaV2,
}

#[derive(Debug, Error)]
pub enum ManagedEnvironmentError {
    #[error("failed to open floxmeta git repo: {0}")]
    OpenFloxmeta(FloxmetaV2Error),
    #[error("failed to fetch environment: {0}")]
    Fetch(GitCommandError),
    #[error("failed to check for git revision: {0}")]
    CheckGitRevision(GitCommandError),
    #[error("can't find local_rev specified in lockfile; local_rev could have been mistakenly committed on another machine")]
    LocalRevDoesNotExist,
    #[error("can't find environment at revision specified in lockfile; this could have been caused by force pushing")]
    RevDoesNotExist,
    #[error("invalid {} file: {0}", GENERATION_LOCK_FILENAME)]
    InvalidLock(serde_json::Error),
    #[error("internal error: {0}")]
    Git(GitCommandError),
    #[error("internal error: {0}")]
    GitBranchHash(GitCommandBranchHashError),
    #[error("couldn't write environment lockfile: {0}")]
    WriteLock(io::Error),
    #[error("couldn't serialize environment lockfile: {0}")]
    SerializeLock(serde_json::Error),
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct GenerationLock {
    rev: String,
    local_rev: Option<String>,
    version: Version<1>,
}

#[async_trait]
impl Environment for ManagedEnvironment {
    #[allow(unused)]
    async fn build(
        &mut self,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<(), EnvironmentError2> {
        todo!()
    }

    /// Install packages to the environment atomically
    #[allow(unused)]
    async fn install(
        &mut self,
        packages: Vec<FloxPackage>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<bool, EnvironmentError2> {
        todo!()
    }

    /// Uninstall packages from the environment atomically
    #[allow(unused)]
    async fn uninstall(
        &mut self,
        packages: Vec<FloxPackage>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<bool, EnvironmentError2> {
        todo!()
    }

    /// Atomically edit this environment, ensuring that it still builds
    #[allow(unused)]
    async fn edit(
        &mut self,
        nix: &NixCommandLine,
        system: System,
        contents: String,
    ) -> Result<(), EnvironmentError2> {
        todo!()
    }

    /// Extract the current content of the manifest
    fn manifest_content(&self) -> Result<String, EnvironmentError2> {
        todo!()
    }

    #[allow(unused)]
    async fn catalog(
        &self,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<EnvCatalog, EnvironmentError2> {
        todo!()
    }

    /// Return the [EnvironmentRef] for the environment for identification
    #[allow(unused)]
    fn environment_ref(&self) -> EnvironmentRef {
        todo!()
    }

    #[allow(unused)]
    fn flake_attribute(&self, _system: System) -> FlakeAttribute {
        todo!()
    }

    /// Returns the environment owner
    #[allow(unused)]
    fn owner(&self) -> Option<EnvironmentOwner> {
        todo!()
    }

    /// Returns the environment name
    #[allow(unused)]
    fn name(&self) -> EnvironmentName {
        todo!()
    }

    /// Delete the Environment
    #[allow(unused)]
    fn delete(self) -> Result<(), EnvironmentError2> {
        todo!()
    }
}

impl ManagedEnvironment {
    pub fn encode(pointer: &ManagedPointer, _path: impl AsRef<Path>) -> String {
        format!("todo-{}", pointer.name)
    }

    /// Open a managed environment by reading its lockfile and ensuring there is
    /// a unique branch to track its state in floxmeta.
    ///
    /// The definition of a managed environment is stored on a branch in a
    /// central clone of the environment owner's floxmeta repository located in
    /// `$FLOX_DATA_DIR/meta/<owner>`. Every .flox directory will correspond to
    /// a unique branch.
    ///
    /// To open an environment at a given location:
    ///
    /// - We open a user's floxmeta clone, i.e. `$FLOX_DATA_DIR/meta/<owner>`.
    ///   If that repo doesn't exist, it will be cloned.
    ///
    /// - We open the lockfile and ensure that the specific commit referenced is
    ///   present in floxmeta. If it is not, we fetch the environment and lock
    ///   to `HEAD`.
    ///
    /// - We check whether a unique branch for the .flox directory exists in
    ///   floxmeta and points at the commit in the lockfile, creating or
    ///   resetting the correct branch if necessary.
    ///
    /// At some point, it may be useful to create a ManagedEnvironment without
    /// fetching or cloning. This would be more correct for commands like delete
    /// that don't need to fetch the environment.
    pub fn open(
        flox: &Flox,
        pointer: ManagedPointer,
        dot_flox_path: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError2> {
        let floxmeta =
            FloxmetaV2::open(flox, &pointer).map_err(ManagedEnvironmentError::OpenFloxmeta)?;

        let lock = Self::ensure_locked(flox, &pointer, &dot_flox_path, &floxmeta)?;
        Self::ensure_branch(
            &branch_name(&flox.system, &pointer, &dot_flox_path),
            &lock,
            &floxmeta,
        )?;

        Ok(ManagedEnvironment {
            _path: dot_flox_path.as_ref().to_path_buf(),
            _system: flox.system.clone(),
            _floxmeta: floxmeta,
            _pointer: pointer,
        })
    }

    /// Ensure:
    /// - a lockfile exists
    /// - the commit in the lockfile (`local_rev` or `rev``) exists in floxmeta
    ///
    /// This may perform a fetch.
    fn ensure_locked(
        flox: &Flox,
        pointer: &ManagedPointer,
        dot_flox_path: impl AsRef<Path>,
        floxmeta: &FloxmetaV2,
    ) -> Result<GenerationLock, EnvironmentError2> {
        let lock_path = dot_flox_path.as_ref().join(GENERATION_LOCK_FILENAME);
        let maybe_lock: Option<GenerationLock> = match fs::read(&lock_path) {
            Ok(lock_contents) => Some(
                serde_json::from_slice(&lock_contents)
                    .map_err(ManagedEnvironmentError::InvalidLock)?,
            ),
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => None,
                _ => Err(EnvironmentError2::ReadManifest(err))?,
            },
        };

        Ok(match maybe_lock {
            // Use local_rev if we have it
            Some(lock) if lock.local_rev.is_some() => {
                // Because a single floxmeta clone contains multiple
                // environments, `local_rev` might refer to a commit on a
                // branch for another environment. We protect against this
                // for `rev` below, but it doesn't seem worth doing so for
                // `local_rev`, because the environment directory may have
                // been moved. This means we can't require the commit is on
                // the {system}.{name}.{encode(project dir)} branch. We
                // could require the commit to be on some {system}.{name}.*
                // branch, but that doesn't seem worth the effort.
                if !floxmeta
                    .git
                    // we know local_rev is Some because of the above match
                    .contains_commit(lock.local_rev.as_ref().unwrap())
                    .map_err(ManagedEnvironmentError::CheckGitRevision)?
                {
                    Err(ManagedEnvironmentError::LocalRevDoesNotExist)?;
                }
                lock
            },
            // We have rev but not local_rev
            Some(lock) => {
                let remote_branch = remote_branch_name(&flox.system, pointer);
                // Check that the commit not only exists but is on the
                // correct branch - we don't want to allow grabbing commits
                // from other environments.
                if !floxmeta
                    .git
                    .branch_contains_commit(&lock.rev, &remote_branch)
                    .map_err(ManagedEnvironmentError::Git)?
                {
                    // Maybe the lock refers to a new generation that has
                    // been pushed, so fetch. We fetch the branch rather
                    // than the rev because we don't want to grab a commit
                    // from another environment.
                    floxmeta
                        .git
                        .fetch_branch("origin", &remote_branch)
                        .map_err(|err| match err {
                            GitCommandError::BadExit(_, _, _) => {
                                ManagedEnvironmentError::Fetch(err)
                            },
                            _ => ManagedEnvironmentError::Git(err),
                        })?;
                }
                if !floxmeta
                    .git
                    .branch_contains_commit(&lock.rev, &remote_branch)
                    .map_err(ManagedEnvironmentError::Git)?
                {
                    Err(ManagedEnvironmentError::RevDoesNotExist)?;
                };
                lock
            },
            // There's no lockfile, so write a new one with whatever remote
            // branch is after fetching.
            None => {
                let remote_branch = remote_branch_name(&flox.system, pointer);
                floxmeta
                    .git
                    .fetch_branch("origin", &remote_branch)
                    .map_err(ManagedEnvironmentError::Fetch)?;
                let rev = floxmeta
                    .git
                    .branch_hash(&remote_branch)
                    .map_err(ManagedEnvironmentError::GitBranchHash)?;
                let lock = GenerationLock {
                    rev,
                    local_rev: None,
                    version: Version::<1> {},
                };
                let lock_contents = serde_json::to_string_pretty(&lock)
                    .map_err(ManagedEnvironmentError::SerializeLock)?;
                debug!("writing rev '{}' to lockfile", lock.rev);
                fs::write(&lock_path, lock_contents).map_err(ManagedEnvironmentError::WriteLock)?;
                lock
            },
        })
    }

    /// Ensure the branch exists and points at rev or local_rev
    fn ensure_branch(
        branch: &str,
        lock: &GenerationLock,
        floxmeta: &FloxmetaV2,
    ) -> Result<(), ManagedEnvironmentError> {
        let current_rev = lock.local_rev.as_ref().unwrap_or(&lock.rev);
        match floxmeta.git.branch_hash(branch) {
            Ok(ref branch_rev) => {
                if branch_rev != current_rev {
                    // Maybe the user pulled a new lockfile or there was a race with
                    // another `flox` process and the ManagedLock has now been
                    // updated.
                    // TODO need to clarify the meaning of the branch name and what
                    // guarantees it represents
                    // For now just point the branch at current_rev.
                    // We're not discarding work, just allowing it to possibly be
                    // garbage collected.
                    floxmeta
                        .git
                        .reset_branch(branch, current_rev)
                        .map_err(ManagedEnvironmentError::Git)?;
                }
            },
            // create branch if it doesn't exist
            Err(GitCommandBranchHashError::DoesNotExist) => {
                floxmeta
                    .git
                    .create_branch(branch, current_rev)
                    .map_err(ManagedEnvironmentError::Git)?;
            },
            Err(err) => Err(ManagedEnvironmentError::GitBranchHash(err))?,
        }
        Ok(())
    }
}

fn branch_name(system: &str, pointer: &ManagedPointer, dot_flox_path: impl AsRef<Path>) -> String {
    format!(
        "{}.{}.{}",
        system,
        pointer.name,
        ManagedEnvironment::encode(pointer, dot_flox_path)
    )
}

pub fn remote_branch_name(system: &str, pointer: &ManagedPointer) -> String {
    format!("{}.{}", system, pointer.name)
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use once_cell::sync::Lazy;

    use super::*;
    use crate::flox::tests::flox_instance;
    use crate::models::environment::{DOT_FLOX, ENVIRONMENT_POINTER_FILENAME};
    use crate::models::floxmetav2::floxmeta_dir;
    use crate::providers::git::tests::commit_file;
    use crate::providers::git::{GitCommandProvider, GitProvider};

    static TEST_POINTER: Lazy<ManagedPointer> = Lazy::new(|| ManagedPointer {
        owner: EnvironmentOwner::from_str("owner").unwrap(),
        name: EnvironmentName::from_str("name").unwrap(),
        version: Version::<1> {},
    });

    fn create_dot_flox(
        dot_flox_path: &Path,
        pointer: &ManagedPointer,
        lock: Option<&GenerationLock>,
    ) {
        fs::create_dir(dot_flox_path).unwrap();
        let pointer_path = dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME);
        fs::write(
            pointer_path,
            serde_json::to_string_pretty(&pointer).unwrap(),
        )
        .unwrap();
        if let Some(lock) = lock {
            let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
            fs::write(lock_path, serde_json::to_string_pretty(lock).unwrap()).unwrap();
        }
    }

    async fn create_floxmeta(flox: &Flox, remote_path: &Path, branch: &str) -> FloxmetaV2 {
        let user_floxmeta_dir = floxmeta_dir(flox, &TEST_POINTER.owner);
        fs::create_dir_all(&user_floxmeta_dir).unwrap();
        GitCommandProvider::clone_branch(
            format!("file://{}", remote_path.to_string_lossy()),
            user_floxmeta_dir,
            branch,
            true,
        )
        .unwrap();

        FloxmetaV2::open(flox, &TEST_POINTER).unwrap()
    }

    /// Test that when ensure_locked has input state of:
    /// - no lock
    /// - floxmeta at commit 1
    /// - remote at commit 2
    ///
    /// It results in output state of:
    /// - lock at commit 2
    /// - floxmeta at commit 2
    #[tokio::test]
    async fn test_ensure_locked_case_1() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_path = flox.temp_dir.join("remote");
        fs::create_dir(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).await.unwrap();

        let branch = remote_branch_name(&flox.system, &TEST_POINTER);
        remote.checkout(&branch, true).await.unwrap();
        commit_file(&remote, "file 1").await;

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &branch).await;

        // add a second commit to the remote
        commit_file(&remote, "file 2").await;
        let hash_2 = remote.branch_hash(&branch).unwrap();

        // create a .flox directory
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        create_dot_flox(&dot_flox_path, &TEST_POINTER, None);

        ManagedEnvironment::ensure_locked(&flox, &TEST_POINTER, &dot_flox_path, &floxmeta).unwrap();

        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
        let lock: GenerationLock = serde_json::from_slice(&fs::read(lock_path).unwrap()).unwrap();
        assert_eq!(lock, GenerationLock {
            rev: hash_2.clone(),
            local_rev: None,
            version: Version::<1> {},
        });

        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_2);
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at commit 1
    /// - floxmeta at commit 1
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - lock at commit 1
    /// - floxmeta at commit 1
    #[tokio::test]
    async fn test_ensure_locked_case_2() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_path = flox.temp_dir.join("remote");
        fs::create_dir(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).await.unwrap();

        let branch = remote_branch_name(&flox.system, &TEST_POINTER);
        remote.checkout(&branch, true).await.unwrap();
        commit_file(&remote, "file 1").await;
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &branch).await;

        // create a .flox directory
        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: None,
            version: Version::<1> {},
        };
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        create_dot_flox(&dot_flox_path, &TEST_POINTER, Some(&lock));

        ManagedEnvironment::ensure_locked(&flox, &TEST_POINTER, &dot_flox_path, &floxmeta).unwrap();

        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
        let lock: GenerationLock = serde_json::from_slice(&fs::read(lock_path).unwrap()).unwrap();
        assert_eq!(lock, GenerationLock {
            rev: hash_1.clone(),
            local_rev: None,
            version: Version::<1> {},
        });

        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_1);
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at commit 2
    /// - floxmeta at commit 1
    /// - remote at commit 3
    ///
    /// It results in output state of:
    /// - lock at commit 2
    /// - floxmeta at commit 3
    #[tokio::test]
    async fn test_ensure_locked_case_3() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_path = flox.temp_dir.join("remote");
        fs::create_dir(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).await.unwrap();

        let branch = remote_branch_name(&flox.system, &TEST_POINTER);
        remote.checkout(&branch, true).await.unwrap();
        commit_file(&remote, "file 1").await;

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &branch).await;

        // add a second commit to the remote
        commit_file(&remote, "file 2").await;
        let hash_2 = remote.branch_hash(&branch).unwrap();

        // add a third commit to the remote
        commit_file(&remote, "file 3").await;
        let hash_3 = remote.branch_hash(&branch).unwrap();

        // create a .flox directory
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let lock = GenerationLock {
            rev: hash_2.clone(),
            local_rev: None,
            version: Version::<1> {},
        };
        create_dot_flox(&dot_flox_path, &TEST_POINTER, Some(&lock));

        ManagedEnvironment::ensure_locked(&flox, &TEST_POINTER, &dot_flox_path, &floxmeta).unwrap();

        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
        let lock: GenerationLock = serde_json::from_slice(&fs::read(lock_path).unwrap()).unwrap();
        assert_eq!(lock, GenerationLock {
            rev: hash_2.clone(),
            local_rev: None,
            version: Version::<1> {},
        });

        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_3);
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at branch_2
    /// - floxmeta at branch_1
    /// - remote at branch_1
    /// - branch_2 present on remote
    ///
    /// It results in output state of:
    /// - error
    #[tokio::test]
    async fn test_ensure_locked_case_4() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_path = flox.temp_dir.join("remote");
        fs::create_dir(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).await.unwrap();

        let branch = remote_branch_name(&flox.system, &TEST_POINTER);
        remote.checkout(&branch, true).await.unwrap();
        commit_file(&remote, "file 1").await;

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &branch).await;

        // add a second branch to the remote
        remote.checkout("branch_2", true).await.unwrap();
        commit_file(&remote, "file 2").await;
        let hash_2 = remote.branch_hash("branch_2").unwrap();

        // create a .flox directory
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let lock = GenerationLock {
            rev: hash_2.clone(),
            local_rev: None,
            version: Version::<1> {},
        };
        create_dot_flox(&dot_flox_path, &TEST_POINTER, Some(&lock));

        assert!(matches!(
            ManagedEnvironment::ensure_locked(&flox, &TEST_POINTER, &dot_flox_path, &floxmeta),
            Err(EnvironmentError2::ManagedEnvironment(
                ManagedEnvironmentError::RevDoesNotExist
            ))
        ));
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at nonexistent commit
    /// - floxmeta at commit 1
    /// - remote at commit 2
    ///
    /// It results in output state of:
    /// - error
    #[tokio::test]
    async fn test_ensure_locked_case_5() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_path = flox.temp_dir.join("remote");
        fs::create_dir(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).await.unwrap();

        let branch = remote_branch_name(&flox.system, &TEST_POINTER);
        remote.checkout(&branch, true).await.unwrap();
        commit_file(&remote, "file 1").await;

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &branch).await;

        // add a second commit to the remote
        commit_file(&remote, "file 2").await;
        let hash_2 = remote.branch_hash(&branch).unwrap();

        // create a .flox directory
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let lock = GenerationLock {
            rev: "does not exist".to_string(),
            local_rev: None,
            version: Version::<1> {},
        };
        create_dot_flox(&dot_flox_path, &TEST_POINTER, Some(&lock));

        assert!(matches!(
            ManagedEnvironment::ensure_locked(&flox, &TEST_POINTER, &dot_flox_path, &floxmeta),
            Err(EnvironmentError2::ManagedEnvironment(
                ManagedEnvironmentError::RevDoesNotExist
            ))
        ));

        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_2);
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at {rev: commit 1, local_rev: commit 1}
    /// - floxmeta at commit 1
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - no change
    #[tokio::test]
    async fn test_ensure_locked_case_6() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_path = flox.temp_dir.join("remote");
        fs::create_dir(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).await.unwrap();

        let branch = remote_branch_name(&flox.system, &TEST_POINTER);
        remote.checkout(&branch, true).await.unwrap();
        commit_file(&remote, "file 1").await;
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &branch).await;

        // create a .flox directory
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some(hash_1.clone()),
            version: Version::<1> {},
        };
        create_dot_flox(&dot_flox_path, &TEST_POINTER, Some(&lock));

        ManagedEnvironment::ensure_locked(&flox, &TEST_POINTER, &dot_flox_path, &floxmeta).unwrap();

        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);

        assert_eq!(
            lock,
            serde_json::from_slice(&fs::read(lock_path).unwrap()).unwrap()
        );

        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_1);
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at {rev: commit 1, local_rev: does not exist}
    /// - floxmeta at commit 1
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - error
    #[tokio::test]
    async fn test_ensure_locked_case_7() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_path = flox.temp_dir.join("remote");
        fs::create_dir(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).await.unwrap();

        let branch = remote_branch_name(&flox.system, &TEST_POINTER);
        remote.checkout(&branch, true).await.unwrap();
        commit_file(&remote, "file 1").await;
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &branch).await;

        // create a .flox directory
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some("does not exist".to_string()),
            version: Version::<1> {},
        };
        create_dot_flox(&dot_flox_path, &TEST_POINTER, Some(&lock));

        assert!(matches!(
            ManagedEnvironment::ensure_locked(&flox, &TEST_POINTER, &dot_flox_path, &floxmeta),
            Err(EnvironmentError2::ManagedEnvironment(
                ManagedEnvironmentError::LocalRevDoesNotExist
            ))
        ));
    }

    /// Test that ensure_branch is a no-op with input state:
    /// - branch at commit 1
    /// - rev at commit 1
    /// - local_rev at commit 1
    #[tokio::test]
    async fn test_ensure_branch_noop() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_path = flox.temp_dir.join("remote");
        fs::create_dir(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).await.unwrap();

        let branch = remote_branch_name(&flox.system, &TEST_POINTER);
        remote.checkout(&branch, true).await.unwrap();
        commit_file(&remote, "file 1").await;
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &branch).await;

        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some(hash_1.clone()),
            version: Version::<1> {},
        };
        ManagedEnvironment::ensure_branch(&branch, &lock, &floxmeta).unwrap();
        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_1);
    }

    /// Test that with input state:
    /// - branch at commit 1
    /// - rev at commit 1
    /// - local_rev at commit 2
    /// ensure_branch resets the branch to commit 2
    #[tokio::test]
    async fn test_ensure_branch_resets_branch() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_path = flox.temp_dir.join("remote");
        fs::create_dir(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).await.unwrap();

        let branch = remote_branch_name(&flox.system, &TEST_POINTER);
        remote.checkout(&branch, true).await.unwrap();
        commit_file(&remote, "file 1").await;
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // add a second branch to the remote
        remote.checkout("branch_2", true).await.unwrap();
        commit_file(&remote, "file 2").await;
        let hash_2 = remote.branch_hash("branch_2").unwrap();

        // Create a mock floxmeta (note clone is used instead of clone_branch,
        // which is used in create_floxmeta, because we need both branches)
        let user_floxmeta_dir = floxmeta_dir(&flox, &TEST_POINTER.owner);
        fs::create_dir_all(&user_floxmeta_dir).unwrap();
        <GitCommandProvider as GitProvider>::clone(
            format!("file://{}", remote_path.to_string_lossy()),
            user_floxmeta_dir,
            true,
        )
        .await
        .unwrap();

        let floxmeta = FloxmetaV2::open(&flox, &TEST_POINTER).unwrap();

        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some(hash_2.clone()),
            version: Version::<1> {},
        };
        ManagedEnvironment::ensure_branch(&branch, &lock, &floxmeta).unwrap();
        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_2);
    }

    /// Test that with input state:
    /// - branch_2 does not exist
    /// - rev at commit 1
    /// - local_rev at commit 1
    /// ensure_branch creates branch_2 at commit 1
    #[tokio::test]
    async fn test_ensure_branch_creates_branch() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_path = flox.temp_dir.join("remote");
        fs::create_dir(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).await.unwrap();

        let branch = remote_branch_name(&flox.system, &TEST_POINTER);
        remote.checkout(&branch, true).await.unwrap();
        commit_file(&remote, "file 1").await;
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &branch).await;

        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some(hash_1.clone()),
            version: Version::<1> {},
        };
        ManagedEnvironment::ensure_branch("branch_2", &lock, &floxmeta).unwrap();
        assert_eq!(floxmeta.git.branch_hash("branch_2").unwrap(), hash_1);
    }
}
