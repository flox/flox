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
    async fn init_repo(&self) -> Result<()>;
    async fn add_remote(&self) -> Result<()>;
    /// Move a file from one path to another using git.
    async fn mv(&self, from: &Path, to: &Path) -> Result<()>;
}

#[derive(Copy, Clone)]
pub struct GitCommandProvider;

#[async_trait]
impl GitProvider for GitCommandProvider {
    async fn init_repo(&self) -> Result<()> {
        let process = Command::new("git")
            .arg("init")                   
            .output();
        
        
        let output = process.await?;

        Ok(())
    }

    async fn add_remote(&self) -> Result<()> {
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