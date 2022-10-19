use std::path::Path;

use anyhow::Result;
use tokio::process::Command;
use async_trait::async_trait;

pub struct Repository {
    name: String,
    path: String,
    remote: String
}
// simple git provider for the tasks we need to provide in
// flox
#[async_trait]
pub trait GitProvider {
    /// Example of how to do a DI approach to git providers
    async fn get_provider() -> Result<Box<dyn GitProvider>> where Self: Sized {
        let git_provider = crate::config::CONFIG.read()
            .await.get("GIT_PROVIDER")?;
        match git_provider {
            _ => Ok(Box::new(GitCommandProvider))
        }
    }
    async fn doctor(&self) -> bool ;
    async fn init_repo(&self) -> Result<()>;
    async fn add_remote(&self,origin_name: &str, url: &str) -> Result<()>;
    /// Move a file from one path to another using git.
    async fn mv(&self, from: &Path, to: &Path) -> Result<()>;
}

#[derive(Copy, Clone)]
pub struct GitCommandProvider;

/// A simple Git Provider that uses the git 
/// command. This would require that git is installed.
#[async_trait]
impl GitProvider for GitCommandProvider { 
    async fn doctor(&self) -> bool {
        // look for git command in the path
        if !Command::new("git").arg("--help").output().await.is_ok() {
            error!("Could not find git in the path.");
            return false;
        }

        true
    }
    async fn init_repo(&self) -> Result<()> {
        let process = Command::new("git")
            .arg("init")                   
            .output();
        
        
        let output = process.await?;

        Ok(())
    }

    async fn add_remote(&self, origin_name: &str, url: &str) -> Result<()> {
        let process = Command::new("git")
            .arg("remote")
            .arg("add")
            .arg(origin_name)
            .arg(url)         
            .output();
        
        
        let output = process.await?;
        Ok(())
    }
    
    async fn mv(&self,from: &Path, to: &Path) -> Result<()> {
        let process = Command::new("git")
        .arg("mv")                   
        .arg(format!("{}", from.as_os_str().to_string_lossy()))
        .arg(format!("{}", to.as_os_str().to_string_lossy()))
        .output();
    
    
        let output = process.await?;

        Ok(())
    }
}

pub type DefaultGitProvider = GitCommandProvider;