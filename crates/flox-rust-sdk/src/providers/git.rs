use std::ffi::OsString;
use std::fmt;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use log::error;
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
    type AddRemoteError: std::error::Error;
    type MvError: std::error::Error;
    type RmError: std::error::Error;
    type AddError: std::error::Error;
    type ListBranchesError: std::error::Error;
    type ShowError: std::error::Error + Send + Sync + 'static;
    type DiscoverError: std::error::Error
        + GitDiscoverError
        + Send
        + Sync
        + std::fmt::Debug
        + 'static;
    type FetchError: std::error::Error;

    async fn discover<P: AsRef<Path>>(path: P) -> Result<Self, Self::DiscoverError>;
    async fn init<P: AsRef<Path>>(path: P, bare: bool) -> Result<Self, Self::InitError>;
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
    async fn show(&self, object: &str) -> Result<OsString, Self::ShowError>;
    async fn list_branches(&self) -> Result<Vec<BranchInfo>, Self::ListBranchesError>;

    async fn fetch(&self) -> Result<(), Self::FetchError>;

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
    type DiscoverError = git2::Error;
    type FetchError = EmptyError;
    type InitError = git2::Error;
    type ListBranchesError = EmptyError;
    type MvError = EmptyError;
    type RmError = EmptyError;
    type ShowError = EmptyError;

    async fn init<P: AsRef<Path>>(path: P, bare: bool) -> Result<LibGit2Provider, Self::InitError> {
        Ok(LibGit2Provider {
            repository: if bare {
                git2::Repository::init_bare(path)?
            } else {
                git2::Repository::init(path)?
            },
        })
    }

    fn workdir(&self) -> Option<&Path> {
        self.repository.workdir()
    }

    fn path(&self) -> &Path {
        self.repository.path()
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

    async fn show(&self, _object: &str) -> Result<OsString, Self::ShowError> {
        todo!()
    }

    async fn list_branches(&self) -> Result<Vec<BranchInfo>, Self::ListBranchesError> {
        todo!()
    }

    async fn fetch(&self) -> Result<(), Self::FetchError> {
        todo!()
    }

    async fn discover<P: AsRef<Path>>(path: P) -> Result<Self, Self::DiscoverError> {
        Ok(LibGit2Provider {
            repository: git2::Repository::discover(path)?,
        })
    }
}

#[derive(Error, Debug)]
pub enum GitCommandError {
    #[error("Failed to run git: {0}")]
    Command(#[from] std::io::Error),
    #[error("Git failed with: [exit code {0}]\n{1}")]
    BadExit(i32, String),
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
        let out = command.output().await?;

        if !out.status.success() {
            return Err(GitCommandError::BadExit(
                out.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stderr).to_string(),
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
    type DiscoverError = GitCommandDiscoverError;
    type FetchError = GitCommandError;
    type InitError = GitCommandError;
    type ListBranchesError = GitCommandError;
    type MvError = GitCommandError;
    type RmError = GitCommandError;
    type ShowError = GitCommandError;

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

    fn workdir(&self) -> Option<&Path> {
        self.workdir.as_ref().map(|x| x.as_ref())
    }

    fn path(&self) -> &Path {
        self.path.as_ref()
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

    async fn show(&self, object: &str) -> Result<OsString, Self::ShowError> {
        let mut command = GitCommandProvider::new_command(&self.workdir);
        command.arg("show");
        command.arg(object);

        Ok(GitCommandProvider::run_command(&mut command).await?)
    }

    async fn list_branches(&self) -> Result<Vec<BranchInfo>, Self::ListBranchesError> {
        let mut command = GitCommandProvider::new_command(&self.workdir);
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

    async fn discover<P: AsRef<Path>>(path: P) -> Result<Self, Self::DiscoverError> {
        let out = GitCommandProvider::run_command(
            GitCommandProvider::new_command(&Some(path))
                .arg("rev-parse")
                .arg("--show-toplevel"),
        )
        .await?;

        let out_str = match out.to_str() {
            Some(s) => s,
            None => return Err(GitCommandDiscoverError::GitDirEncoding),
        };

        let workdir = PathBuf::from(out_str.trim());

        Ok(GitCommandProvider {
            workdir: Some(workdir.clone()),
            path: workdir,
        })
    }
}
