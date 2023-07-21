use std::ffi::{OsStr, OsString};
use std::fmt;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use log::{debug, error};
use thiserror::Error;
use tokio::process::Command;

#[derive(Error, Debug)]
pub enum EmptyError {}

pub trait GitDiscoverError {
    fn not_found(&self) -> bool;
}

pub struct BranchInfo {
    pub name: String,
    pub remote: Option<String>,
    pub rev: String,
    pub description: String,
}

// simple git provider for the tasks we need to provide in
// flox
#[async_trait(?Send)]
pub trait GitProvider: Send + Sized + std::fmt::Debug {
    type InitError: std::error::Error;
    type CloneError: std::error::Error;
    type CommitError: std::error::Error;
    type PushError: std::error::Error;

    type CheckoutError: std::error::Error;
    type ListBranchesError: std::error::Error;
    type RenameError: std::error::Error;

    type AddRemoteError: std::error::Error;
    type MvError: std::error::Error;
    type RmError: std::error::Error;
    type AddError: std::error::Error;
    type ShowError: std::error::Error + Send + Sync + 'static;
    type DiscoverError: std::error::Error
        + GitDiscoverError
        + Send
        + Sync
        + std::fmt::Debug
        + 'static;
    type FetchError: std::error::Error;
    type SetOriginError: std::error::Error;
    type GetOriginError: std::error::Error;

    async fn discover<P: AsRef<Path>>(path: P) -> Result<Self, Self::DiscoverError>;
    async fn init<P: AsRef<Path>>(path: P, bare: bool) -> Result<Self, Self::InitError>;
    async fn clone<O: AsRef<OsStr>, P: AsRef<Path>>(
        origin: O,
        path: P,
        bare: bool,
    ) -> Result<Self, Self::CloneError>;

    async fn checkout(&self, name: &str, orphan: bool) -> Result<(), Self::CheckoutError>;
    async fn list_branches(&self) -> Result<Vec<BranchInfo>, Self::ListBranchesError>;
    async fn rename_branch(&self, new_name: &str) -> Result<(), Self::RenameError>;

    async fn add_remote(&self, origin_name: &str, url: &str) -> Result<(), Self::AddRemoteError>;
    async fn mv(&self, from: &Path, to: &Path) -> Result<(), Self::MvError>;
    async fn rm(
        &self,
        paths: &[&Path],
        recursive: bool,
        force: bool,
        cached: bool,
    ) -> Result<(), Self::RmError>;
    async fn add(&self, paths: &[&Path]) -> Result<(), Self::AddError>;
    async fn commit(&self, message: &str) -> Result<(), Self::CommitError>;

    async fn show(&self, object: &str) -> Result<OsString, Self::ShowError>;

    async fn fetch(&self) -> Result<(), Self::FetchError>;
    async fn push(&self, remote: &str) -> Result<(), Self::PushError>;
    async fn set_origin(&self, branch: &str, origin_name: &str)
        -> Result<(), Self::SetOriginError>;

    async fn get_origin(
        &self,
    ) -> Result<(String, String, Option<(String, String)>), Self::GetOriginError>;

    fn workdir(&self) -> Option<&Path>;
    fn path(&self) -> &Path;
}

#[derive(Error, Debug)]
pub enum LibGit2NewError {
    #[error("Error checking current directory: {0}")]
    CurrentDirError(#[from] std::io::Error),
    #[error("Error opening git repostory: {0}")]
    OpenRepositoryError(#[from] git2::Error),
}

pub struct LibGit2Provider {
    repository: git2::Repository,
}

impl fmt::Debug for LibGit2Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibGit2Provider")
            .field("workdir", &self.workdir())
            .finish()
    }
}

impl GitDiscoverError for git2::Error {
    fn not_found(&self) -> bool {
        self.code() == git2::ErrorCode::NotFound
    }
}

#[async_trait(?Send)]
// STUB
impl GitProvider for LibGit2Provider {
    type AddError = EmptyError;
    type AddRemoteError = EmptyError;
    type CheckoutError = EmptyError;
    type CloneError = EmptyError;
    type CommitError = EmptyError;
    type DiscoverError = git2::Error;
    type FetchError = EmptyError;
    type GetOriginError = EmptyError;
    type InitError = git2::Error;
    type ListBranchesError = EmptyError;
    type MvError = EmptyError;
    type PushError = EmptyError;
    type RenameError = EmptyError;
    type RmError = EmptyError;
    type SetOriginError = EmptyError;
    type ShowError = EmptyError;

    async fn discover<P: AsRef<Path>>(path: P) -> Result<Self, Self::DiscoverError> {
        Ok(LibGit2Provider {
            repository: git2::Repository::discover(path)?,
        })
    }

    async fn init<P: AsRef<Path>>(path: P, bare: bool) -> Result<LibGit2Provider, Self::InitError> {
        Ok(LibGit2Provider {
            repository: if bare {
                git2::Repository::init_bare(path)?
            } else {
                git2::Repository::init(path)?
            },
        })
    }

    async fn clone<O: AsRef<OsStr>, P: AsRef<Path>>(
        _origin: O,
        _path: P,
        _bare: bool,
    ) -> Result<Self, Self::CloneError> {
        todo!()
    }

    async fn checkout(&self, _name: &str, _orphan: bool) -> Result<(), Self::CheckoutError> {
        todo!()
    }

    async fn list_branches(&self) -> Result<Vec<BranchInfo>, Self::ListBranchesError> {
        todo!()
    }

    async fn rename_branch(&self, _new_name: &str) -> Result<(), Self::RenameError> {
        todo!()
    }

    async fn add_remote(&self, _origin_name: &str, _url: &str) -> Result<(), Self::AddRemoteError> {
        todo!()
    }

    async fn mv(&self, _from: &Path, _to: &Path) -> Result<(), Self::MvError> {
        todo!()
    }

    async fn rm(
        &self,
        _paths: &[&Path],
        _recursive: bool,
        _force: bool,
        _cached: bool,
    ) -> Result<(), Self::MvError> {
        todo!()
    }

    async fn add(&self, _paths: &[&Path]) -> Result<(), Self::AddError> {
        todo!()
    }

    async fn commit(&self, _message: &str) -> Result<(), Self::CommitError> {
        todo!()
    }

    async fn show(&self, _object: &str) -> Result<OsString, Self::ShowError> {
        todo!()
    }

    async fn fetch(&self) -> Result<(), Self::FetchError> {
        todo!()
    }

    async fn push(&self, _remote: &str) -> Result<(), Self::PushError> {
        todo!()
    }

    async fn set_origin(
        &self,
        _branch: &str,
        _origin_name: &str,
    ) -> Result<(), Self::SetOriginError> {
        todo!()
    }

    async fn get_origin(
        &self,
    ) -> Result<(String, String, Option<(String, String)>), Self::GetOriginError> {
        todo!()
    }

    fn workdir(&self) -> Option<&Path> {
        self.repository.workdir()
    }

    fn path(&self) -> &Path {
        self.repository.path()
    }
}

#[derive(Error, Debug)]
pub enum GitCommandError {
    #[error("Failed to run git: {0}")]
    Command(#[from] std::io::Error),
    #[error("Git failed with: [exit code {0}]\n  stdout: {1}\n  stderr: {2}")]
    BadExit(i32, String, String),
}

#[derive(Clone, Debug)]
pub struct GitCommandProvider {
    workdir: Option<PathBuf>,
    path: PathBuf,
}

impl GitCommandProvider {
    fn new_command<P: AsRef<Path>>(w: &Option<P>) -> Command {
        let mut c = Command::new(env!("GIT_BIN"));

        if let Some(workdir) = w.as_ref() {
            c.arg("-C");
            c.arg(workdir.as_ref());
        }

        c
    }

    async fn run_command(command: &mut Command) -> Result<OsString, GitCommandError> {
        debug!(target: "posix", "{:?}", command.as_std());
        let out = command.output().await?;

        if !out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();

            return Err(GitCommandError::BadExit(
                out.status.code().unwrap_or(-1),
                stdout,
                stderr,
            ));
        }

        Ok(OsString::from_vec(out.stdout))
    }
}

#[derive(Error, Debug)]
pub enum GitCommandDiscoverError {
    #[error(transparent)]
    Command(#[from] GitCommandError),
    #[error("Git directory is not valid unicode")]
    GitDirEncoding,
    #[error("Git returned an uexpected output: {0}")]
    UnexpectedOutput(String),
}

impl GitDiscoverError for GitCommandDiscoverError {
    fn not_found(&self) -> bool {
        match self {
            // TODO: handle errors
            GitCommandDiscoverError::Command(_) => true,
            _ => false,
        }
    }
}

/// A simple Git Provider that uses the git
/// command. This would require that git is installed.
#[async_trait(?Send)]
impl GitProvider for GitCommandProvider {
    type AddError = GitCommandError;
    type AddRemoteError = GitCommandError;
    type CheckoutError = GitCommandError;
    type CloneError = GitCommandError;
    type CommitError = GitCommandError;
    type DiscoverError = GitCommandDiscoverError;
    type FetchError = GitCommandError;
    type GetOriginError = GitCommandError;
    type InitError = GitCommandError;
    type ListBranchesError = GitCommandError;
    type MvError = GitCommandError;
    type PushError = GitCommandError;
    type RenameError = GitCommandError;
    type RmError = GitCommandError;
    type SetOriginError = GitCommandError;
    type ShowError = GitCommandError;

    async fn discover<P: AsRef<Path>>(path: P) -> Result<Self, Self::DiscoverError> {
        let out = GitCommandProvider::run_command(
            GitCommandProvider::new_command(&Some(&path))
                .arg("rev-parse")
                .arg("--is-bare-repository"),
        )
        .await?;

        let out_str = out
            .to_str()
            .ok_or(GitCommandDiscoverError::GitDirEncoding)?;

        let bare = out_str
            .trim()
            .parse::<bool>()
            .map_err(|_| GitCommandDiscoverError::UnexpectedOutput(out_str.to_string()))?;

        if bare {
            return Ok(GitCommandProvider {
                workdir: None,
                path: path.as_ref().to_path_buf(),
            });
        }

        let out = GitCommandProvider::run_command(
            GitCommandProvider::new_command(&Some(&path))
                .arg("rev-parse")
                .arg("--show-toplevel"),
        )
        .await?;

        let out_str = out
            .to_str()
            .ok_or(GitCommandDiscoverError::GitDirEncoding)?;

        let workdir = PathBuf::from(out_str.trim());

        Ok(GitCommandProvider {
            workdir: Some(workdir.clone()),
            path: workdir,
        })
    }

    async fn init<P: AsRef<Path>>(
        path: P,
        bare: bool,
    ) -> Result<GitCommandProvider, Self::InitError> {
        let mut command = GitCommandProvider::new_command(&Some(&path));
        command.arg("init");
        if bare {
            command.arg("--bare");
        }

        let _out = GitCommandProvider::run_command(&mut command).await?;

        Ok(GitCommandProvider {
            workdir: Some(path.as_ref().into()),
            path: path.as_ref().into(),
        })
    }

    async fn clone<O: AsRef<OsStr>, P: AsRef<Path>>(
        origin: O,
        path: P,
        bare: bool,
    ) -> Result<Self, Self::CloneError> {
        let mut command = GitCommandProvider::new_command(&Some(&path));
        command.arg("clone");
        if bare {
            command.arg("--bare");
        }

        command.arg(origin.as_ref());
        command.arg("./");

        let _out = GitCommandProvider::run_command(&mut command).await?;
        Ok(GitCommandProvider {
            workdir: (!bare).then(|| path.as_ref().to_path_buf()),
            path: path.as_ref().into(),
        })
    }

    async fn checkout(&self, name: &str, orphan: bool) -> Result<(), Self::CheckoutError> {
        let mut command = GitCommandProvider::new_command(&self.workdir());
        command.arg("checkout");
        if orphan {
            command.arg("--orphan");
        }

        command.arg(name);

        let _out = GitCommandProvider::run_command(&mut command).await?;
        Ok(())
    }

    async fn add_remote(&self, origin_name: &str, url: &str) -> Result<(), Self::AddRemoteError> {
        let _out = GitCommandProvider::run_command(
            GitCommandProvider::new_command(&self.workdir)
                .arg("remote")
                .arg("add")
                .arg(origin_name)
                .arg(url),
        )
        .await?;

        Ok(())
    }

    async fn rename_branch(&self, new_name: &str) -> Result<(), Self::RenameError> {
        let _out = GitCommandProvider::run_command(
            GitCommandProvider::new_command(&self.workdir)
                .arg("branch")
                .arg("-m")
                .arg(new_name),
        )
        .await?;
        Ok(())
    }

    async fn set_origin(
        &self,
        branch: &str,
        origin_name: &str,
    ) -> Result<(), Self::SetOriginError> {
        let _out = GitCommandProvider::run_command(
            GitCommandProvider::new_command(&self.workdir)
                .arg("branch")
                .arg(branch)
                .arg("--set-upstream")
                .arg(format!("{origin_name}/{branch}")),
        )
        .await?;

        Ok(())
    }

    /// Retrieve information about the remot origin for the current branch/repo
    ///
    /// Return a tuple containing
    ///
    /// 1. the remote name of the current branch (or "origin" if no upstream configured)
    /// 2. the remote url
    /// 3. (if configured) a tuple containing
    ///    1. the upstream branch name
    ///    2. the current revision of the branch
    async fn get_origin(
        &self,
    ) -> Result<(String, String, Option<(String, String)>), Self::GetOriginError> {
        let (remote_name, remote_branch) = {
            let reference = GitCommandProvider::run_command(
                GitCommandProvider::new_command(&self.workdir)
                    .arg("rev-parse")
                    .arg("--abbrev-ref")
                    .arg("--symbolic-full-name")
                    .arg("@{u}"),
            )
            .await;

            match reference {
                Err(_) => {
                    error!("Couldn't determine upstream remote name for the current branch, defaulting to 'origin'");
                    ("origin".to_string(), None)
                },
                Ok(reference) => reference
                    .to_string_lossy()
                    .split_once('/')
                    .map(|(name, branch)| (name.to_string(), Some(branch.to_string())))
                    .unwrap(),
            }
        };

        let url = GitCommandProvider::run_command(
            GitCommandProvider::new_command(&self.workdir)
                .arg("remote")
                .arg("get-url")
                .arg(&remote_name),
        )
        .await?
        .to_string_lossy()
        .to_string();

        let branch_and_commit = match remote_branch {
            Some(branch) => Some((
                branch.clone(),
                GitCommandProvider::run_command(
                    GitCommandProvider::new_command(&self.workdir)
                        .arg("ls-remote")
                        .arg(&remote_name)
                        .arg(&branch),
                )
                .await?
                .to_string_lossy()[0..40]
                    .to_string(),
            )),
            None => None,
        };

        Ok((remote_name.to_string(), url, branch_and_commit))
    }

    async fn mv(&self, from: &Path, to: &Path) -> Result<(), Self::MvError> {
        let _out = GitCommandProvider::run_command(
            GitCommandProvider::new_command(&self.workdir)
                .arg("mv")
                .arg(format!("{}", from.as_os_str().to_string_lossy()))
                .arg(format!("{}", to.as_os_str().to_string_lossy())),
        )
        .await?;

        Ok(())
    }

    async fn rm(
        &self,
        paths: &[&Path],
        recursive: bool,
        force: bool,
        cached: bool,
    ) -> Result<(), Self::MvError> {
        let mut command = GitCommandProvider::new_command(&self.workdir);

        command.arg("rm");

        if recursive {
            command.arg("-r");
        }
        if force {
            command.arg("--force");
        }
        if cached {
            command.arg("--cached");
        }

        for path in paths {
            command.arg(format!("{}", path.as_os_str().to_string_lossy()));
        }

        let _out = GitCommandProvider::run_command(&mut command).await?;

        Ok(())
    }

    async fn add(&self, paths: &[&Path]) -> Result<(), Self::MvError> {
        let mut command = GitCommandProvider::new_command(&self.workdir);
        command.arg("add");
        for path in paths {
            command.arg(path);
        }

        let _out = GitCommandProvider::run_command(&mut command).await?;

        Ok(())
    }

    async fn commit(&self, message: &str) -> Result<(), Self::CommitError> {
        let mut command = GitCommandProvider::new_command(&self.workdir());
        command.arg("commit");
        command.args(["-m", message]);

        let _out = GitCommandProvider::run_command(&mut command).await?;
        Ok(())
    }

    async fn show(&self, object: &str) -> Result<OsString, Self::ShowError> {
        let mut command = GitCommandProvider::new_command(&Some(&self.path));
        command.arg("show");
        command.arg(object);

        Ok(GitCommandProvider::run_command(&mut command).await?)
    }

    async fn list_branches(&self) -> Result<Vec<BranchInfo>, Self::ListBranchesError> {
        let mut command = GitCommandProvider::new_command(&Some(&self.path));
        command.arg("branch");
        command.args(["--all", "--verbose"]);

        let info = GitCommandProvider::run_command(&mut command)
            .await?
            .to_string_lossy()
            .lines()
            .map(|line| {
                // split all lines into three parts (undoing git's default format)
                // using the `--format` option failed on me to produce any useful output at all
                // If using the git cli that would probably be the better way
                // of providing parseable data.
                //
                // the git putput is formatted as
                // [*] <name> <whitespace> <rev hash> <whitespace> <subject>
                //  L present iff branch is currently checked out

                // the active branch is denoted by a leadinf '*', which cannot be disabled?
                let (full_name, rest) =
                    line.trim_start_matches('*').trim().split_once(' ').unwrap();
                // hash part
                let (hash, rest) = rest.trim().split_once(' ').unwrap();
                // description
                let description = rest.trim_start();
                (full_name, hash, description)
            })
            .map(|(full_name, hash, description)| {
                // discard unknown remotes
                let (remote, name) = match full_name
                    .strip_prefix("remotes/")
                    .and_then(|remote| remote.split_once('/'))
                {
                    Some((remote, name)) => (Some(remote), name),
                    None => (None, full_name),
                };
                BranchInfo {
                    name: name.to_string(),
                    remote: remote.map(String::from),
                    rev: hash.to_string(),
                    description: description.to_string(),
                }
            })
            .collect();

        Ok(info)
    }

    async fn fetch(&self) -> Result<(), Self::FetchError> {
        GitCommandProvider::run_command(
            GitCommandProvider::new_command(&self.workdir.as_deref().or(Some(&self.path)))
                .arg("fetch")
                .arg("--all"),
        )
        .await?;
        Ok(())
    }

    async fn push(&self, remote: &str) -> Result<(), Self::PushError> {
        let mut command = GitCommandProvider::new_command(&self.workdir());
        command.arg("push");
        command.arg(remote);
        command.arg("HEAD");

        let _out = GitCommandProvider::run_command(&mut command).await?;
        Ok(())
    }

    fn workdir(&self) -> Option<&Path> {
        self.workdir.as_ref().map(|x| x.as_ref())
    }

    fn path(&self) -> &Path {
        self.path.as_ref()
    }
}

#[cfg(test)]
pub mod tests {

    use super::*;

    /// A provider with path set to /does-not-exist for use in tests
    pub fn mock_provider() -> GitCommandProvider {
        GitCommandProvider {
            workdir: None,
            path: PathBuf::from("/does-not-exist"),
        }
    }
}
