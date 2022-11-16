use std::path::Path;

use thiserror::Error;

use async_trait::async_trait;
use tokio::process::Command;

pub struct Repository {
    name: String,
    path: String,
    remote: String,
}

// simple git provider for the tasks we need to provide in
// flox
#[async_trait]
pub trait GitProvider {
    type InitError: std::error::Error;
    type AddRemoteError: std::error::Error;
    type MvError: std::error::Error;

    fn new() -> Self;

    /// Example of how to do a DI approach to git providers
    async fn doctor(&self) -> bool;
    async fn init_repo(&self) -> Result<(), Self::InitError>;
    async fn add_remote(&self, origin_name: &str, url: &str) -> Result<(), Self::AddRemoteError>;
    /// Move a file from one path to another using git.
    async fn mv(&self, from: &Path, to: &Path) -> Result<(), Self::MvError>;
}

#[derive(Copy, Clone)]
pub struct GitCommandProvider;

// TODO A provider for LibGit2
#[derive(Copy, Clone)]
pub struct LibGit2Provider;

#[derive(Error, Debug)]
pub enum EmptyError {}

#[async_trait]
// STUB
impl GitProvider for LibGit2Provider {
    type InitError = EmptyError;
    type AddRemoteError = EmptyError;
    type MvError = EmptyError;

    fn new() -> LibGit2Provider {
        LibGit2Provider
    }

    async fn doctor(&self) -> bool {
        todo!()
    }
    async fn init_repo(&self) -> Result<(), EmptyError> {
        todo!()
    }
    async fn add_remote(&self, _origin_name: &str, _url: &str) -> Result<(), EmptyError> {
        todo!()
    }
    async fn mv(&self, _from: &Path, _to: &Path) -> Result<(), EmptyError> {
        todo!()
    }
}

#[derive(Error, Debug)]
pub enum CommandInitError {
    #[error("Error in CLI initializing git repo: {0}")]
    Command(#[from] std::io::Error),
}

#[derive(Error, Debug)]
pub enum CommandAddRemoteError {
    #[error("Error in CLI adding remote repo: {0}")]
    Command(#[from] std::io::Error),
}

#[derive(Error, Debug)]
pub enum CommandMvError {
    #[error("Error in CLI moving file: {0}")]
    Command(#[from] std::io::Error),
}

/// A simple Git Provider that uses the git
/// command. This would require that git is installed.
#[async_trait]
impl GitProvider for GitCommandProvider {
    type InitError = CommandInitError;
    type AddRemoteError = CommandAddRemoteError;
    type MvError = CommandMvError;

    fn new() -> GitCommandProvider {
        GitCommandProvider
    }

    async fn doctor(&self) -> bool {
        // look for git command in the path
        if Command::new("git").arg("--help").output().await.is_err() {
            error!("Could not find git in the path.");
            return false;
        }

        true
    }
    async fn init_repo(&self) -> Result<(), Self::InitError> {
        let process = Command::new("git").arg("init").output();

        let _output = process.await?;

        Ok(())
    }

    async fn add_remote(&self, origin_name: &str, url: &str) -> Result<(), Self::AddRemoteError> {
        let process = Command::new("git")
            .arg("remote")
            .arg("add")
            .arg(origin_name)
            .arg(url)
            .output();

        let _output = process.await?;

        Ok(())
    }

    async fn mv(&self, from: &Path, to: &Path) -> Result<(), Self::MvError> {
        let process = Command::new("git")
            .arg("mv")
            .arg(format!("{}", from.as_os_str().to_string_lossy()))
            .arg(format!("{}", to.as_os_str().to_string_lossy()))
            .output();

        let _output = process.await?;

        Ok(())
    }
}

pub type DefaultGitProvider = GitCommandProvider;
