use std::ffi::OsString;
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

// simple git provider for the tasks we need to provide in
// flox
#[async_trait(?Send)]
pub trait GitProvider: Send + Sized {
    type InitError: std::error::Error;
    type AddRemoteError: std::error::Error;
    type MvError: std::error::Error;
    type RmError: std::error::Error;
    type AddError: std::error::Error;
    type DiscoverError: std::error::Error + GitDiscoverError;

    async fn discover<P: AsRef<Path>>(path: P) -> Result<Self, Self::DiscoverError>;
    async fn init<P: AsRef<Path>>(path: P) -> Result<Self, Self::InitError>;

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

    fn workdir(&self) -> Option<&Path>;
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
    type InitError = git2::Error;
    type MvError = EmptyError;
    type RmError = EmptyError;

    async fn init<P: AsRef<Path>>(path: P) -> Result<LibGit2Provider, Self::InitError> {
        Ok(LibGit2Provider {
            repository: git2::Repository::init(path)?,
        })
    }

    fn workdir(&self) -> Option<&Path> {
        self.repository.workdir()
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
    type InitError = GitCommandError;
    type MvError = GitCommandError;
    type RmError = GitCommandError;

    async fn init<P: AsRef<Path>>(path: P) -> Result<GitCommandProvider, Self::InitError> {
        let _out = GitCommandProvider::run_command(
            GitCommandProvider::new_command(&Some(&path)).arg("init"),
        )
        .await?;

        Ok(GitCommandProvider {
            workdir: Some(path.as_ref().into()),
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

    async fn add(&self, paths: &[&Path]) -> Result<(), Self::MvError> {
        let mut command = GitCommandProvider::new_command(&self.workdir);
        command.arg("add");
        for path in paths {
            command.arg(path);
        }

        let _out = GitCommandProvider::run_command(&mut command).await?;

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

        Ok(GitCommandProvider {
            workdir: Some(PathBuf::from(out_str.trim())),
        })
    }
}
