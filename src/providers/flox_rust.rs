//! Attempt to implement some parts of the bash script in rust
//! 
//! 
//! 
//! 
use std::path::Path;

use crate::models::*;
use async_trait::async_trait;
use anyhow::{anyhow, Result, Ok};
use tokio::process::Command;

use super::git::{DefaultGitProvider, GitProvider, GitCommandProvider};

struct NixCommands {
    
}

impl NixCommands {
    async fn get_templates() -> Result<String> {
        let process = Command::new("nix")
        .arg("eval")
        .arg("--no-write-lock-file")
        .arg("--raw")
        .arg("--apply")
        .arg(r#"
        x: with builtins; concatStringsSep "\n" (
            attrValues (mapAttrs (k: v: k + ": " + v.description) (removeAttrs x ["_init"]))
          )
        ' "flox#templates"
        "#)        
        .output();
    
    
        let output = process.await?;

        Ok(std::str::from_utf8(&output.stdout)?.to_string())
    }
}

struct FloxNativePackageProvider {
    git_provider: Box<dyn GitProvider + Send + Sync>
}

impl FloxNativePackageProvider {
    fn with_command_git() -> Self {
        return FloxNativePackageProvider {
            git_provider: Box::new(GitCommandProvider)
        }
    }
}

///
/// 
#[async_trait]
impl PackageProvider for FloxNativePackageProvider {
    async fn init(&self, package_name: &str, builder: FloxBuilder) -> Result<InitResult> {
    
        if !Path::new("flox.nix").exists() {
            // Init with _init if we haven't already.
            Command::new("nix")
                .arg("flake")
                .arg("init")
                .arg("--template")            
                .arg("flox#templates._init")
            .output().await?;
        }
        
        // create a git repo at this spot
        if !Path::new(".git").exists() {
            self.git_provider.init_repo().await?;
        }

        let mut process = Command::new("nix")
            .arg("flake")
            .arg("init")
            .arg("")            
            .arg(format!("flox#templates.{}", builder))
            .output();
        
        // {
        //     Ok(_) => Ok(CreateResult::new("Package created")),
        //     Err(err) => Err(anyhow!("Error thrown trying to create a message: {}", err)),
        // }
        let output = process.await?;

        // after init we create some structure
        std::fs::create_dir_all(format!("pkgs/{}", package_name))?;
        // move the default.nix into the pkgs directory
        self.git_provider.mv(Path::new("pkgs/default.nix"), 
            Path::new(&format!("pkgs/{}/default.nix", package_name))).await?;

        Ok(InitResult::new(std::str::from_utf8(&output.stdout)?))
    }
    async fn create(&self, package_name: &str) -> Result<CreateResult> {
    
        let mut process = Command::new("flox")
            .arg("create")
            .arg(package_name)
            .output();
        
        // {
        //     Ok(_) => Ok(CreateResult::new("Package created")),
        //     Err(err) => Err(anyhow!("Error thrown trying to create a message: {}", err)),
        // }
        let output = process.await?;

        Ok(CreateResult::new(std::str::from_utf8(&output.stdout)?))
    }

    async fn install(&self) -> Result<InstallResult> {
        Ok(InstallResult{})
    }

    async fn search(&self, query: &str) -> Result<SearchResult> {
        Ok(SearchResult::new(Vec::new()))
    }

    async fn shell(&self) -> Result<()> {
        Ok(())
    }
}
