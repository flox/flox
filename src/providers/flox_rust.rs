//! Attempt to implement some parts of the bash script in rust
//! 
//! 
//! 
//! 
use std::{path::Path, collections::HashMap};
use log::{info, warn, error};

use crate::models::*;
use async_trait::async_trait;
use anyhow::{anyhow, Result};
use tokio::process::Command;

use super::git::{DefaultGitProvider, GitProvider, GitCommandProvider};
use crate::environment::*;
struct FloxRunner {
    
}

impl FloxRunner {
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

    async fn run_in_nix(cmd: &str, args: &Vec<&str>) -> Result<String> {
        let output = Command::new(get_nix_cmd())
                .envs(&build_flox_env()?)
                .arg(cmd)
                .args(args)
                .output().await?;

        let nix_response = std::str::from_utf8(&output.stdout)?;
        let nix_err_response = std::str::from_utf8(&output.stderr)?;

        if !output.stderr.is_empty() {
            error!("Error in nix response, {}", nix_err_response);
            Err(anyhow!("Error in nix response"))
        } else {
            Ok(nix_response.to_string())
        }
    }
    async fn run_in_flox(cmd: &str, args: &Vec<&str>) -> Result<String> {
        let output = Command::new("flox")
                .arg(cmd)
                .args(args)
                .output().await?;

        let nix_response = std::str::from_utf8(&output.stdout)?;
        let nix_err_response = std::str::from_utf8(&output.stderr)?;

        if !output.stderr.is_empty() {
            error!("Error in nix response, {}", nix_err_response);
            Err(anyhow!("Error in nix response"))
        } else {
            Ok(nix_response.to_string())
        }
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
        info!("init {}, {}", package_name, builder);
                    

        if !Path::new("flox.nix").exists() {
            // Init with _init if we haven't already.
            info!("No flox.nix exists, running flox#templates._init");

            let run = FloxRunner::run_in_nix("flake", &vec!["init","--template","flox#templates._init"]).await ;

            match run {
                Ok(response) => info!("Ran flox initialization template. {}", response),
                Err(e) => error!("FXXXX: Error initializing flox: {}",e)
            };
        }
        
        // create a git repo at this spot
        if !Path::new(".git").exists() {
            info!("No git repository locally, creating one");
            self.git_provider.init_repo().await?;
        }
         
        match FloxRunner::run_in_nix("flake",
            &vec!["init","--template", &format!("flox#templates.{}", builder)]).await {            
                Ok(response) => info!("Ran flox builder template. {}", response),
                Err(e) => {
                    error!("FXXXX: Error initializing flox: {}",e);
                    // fatal, 
                    return Err(e);
                }
        };        
        // after init we create some structure
        std::fs::create_dir_all(format!("pkgs/{}", package_name))?;
        // move the default.nix into the pkgs directory
        self.git_provider.mv(Path::new("pkgs/default.nix"), 
            Path::new(&format!("pkgs/{}/default.nix", package_name))).await?;

        Ok(InitResult::new("Done"))
    }
    
    async fn environments(&self) -> Result<Vec<Environment>> {
    
        let mut output = FloxRunner::run_in_flox("environments", &vec![]).await?;

        Ok(vec![])
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

#[cfg(test)]
mod test {
    use anyhow::Result;

    use crate::models::{FloxBuilder, PackageProvider};

    use super::FloxNativePackageProvider;

    #[tokio::test]
    async fn test_init_cmd() -> Result<()> {
        std::env::set_var("RUST_LOG", "info");

        pretty_env_logger::init();

        info!("Logging");
        let pkg_prov = FloxNativePackageProvider::with_command_git();
        
        pkg_prov.init("test_pkg", FloxBuilder::RustPackage).await?;

        Ok(())
    }
}
