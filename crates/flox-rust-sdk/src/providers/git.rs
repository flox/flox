use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use tokio::process::Command;

pub struct Repository {
    name: String,
    path: String,
    remote: String,
}
/// Get the Git provider that is currently configured in the environment
async fn get_provider() -> Result<Box<dyn GitProvider>> {
    let git_provider = crate::config::CONFIG
        .read()
        .await
        .get::<String>("git_provider")?; // ENV: FLOX_GIT_PROVIDER
    Ok(match git_provider.as_str() {
        "command" => Box::new(GitCommandProvider),
        "libgit2" => Box::new(LibGit2Provider),
        _ => Box::new(GitCommandProvider),
    })
}
// simple git provider for the tasks we need to provide in
// flox
#[async_trait]
pub trait GitProvider {
    /// Example of how to do a DI approach to git providers
    async fn doctor(&self) -> bool;
    async fn init_repo(&self) -> Result<()>;
    async fn add_remote(&self, origin_name: &str, url: &str) -> Result<()>;
    /// Move a file from one path to another using git.
    async fn mv(&self, from: &Path, to: &Path) -> Result<()>;
}

#[derive(Copy, Clone)]
pub struct GitCommandProvider;

// TODO A provider for LibGit2
#[derive(Copy, Clone)]
pub struct LibGit2Provider;

#[async_trait]
// STUB
impl GitProvider for LibGit2Provider {
    async fn doctor(&self) -> bool {
        todo!()
    }
    async fn init_repo(&self) -> Result<()> {
        todo!()
    }
    async fn add_remote(&self, _origin_name: &str, _url: &str) -> Result<()> {
        todo!()
    }
    async fn mv(&self, _from: &Path, _to: &Path) -> Result<()> {
        todo!()
    }
}

/// A simple Git Provider that uses the git
/// command. This would require that git is installed.
#[async_trait]
impl GitProvider for GitCommandProvider {
    async fn doctor(&self) -> bool {
        // look for git command in the path
        if Command::new("git").arg("--help").output().await.is_err() {
            error!("Could not find git in the path.");
            return false;
        }

        true
    }
    async fn init_repo(&self) -> Result<()> {
        let process = Command::new("git").arg("init").output();

        let _output = process.await?;

        Ok(())
    }

    async fn add_remote(&self, origin_name: &str, url: &str) -> Result<()> {
        let process = Command::new("git")
            .arg("remote")
            .arg("add")
            .arg(origin_name)
            .arg(url)
            .output();

        let _output = process.await?;
        Ok(())
    }

    async fn mv(&self, from: &Path, to: &Path) -> Result<()> {
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
