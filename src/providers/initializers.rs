//! Attempt to implement some parts of the bash script in rust
//! 
//! 
//! 
//! 
use std::{path::Path, collections::HashMap, process::Stdio};
use log::{info, warn, error};

use crate::{models::*, utils::runner::CommandRunner};
use async_trait::async_trait;
use anyhow::{anyhow, Result};
use tokio::process::Command;

use super::{git::{DefaultGitProvider, GitProvider, GitCommandProvider}};
use crate::environment::*;


#[async_trait]
pub trait Initializer {
    async fn get_provider() -> Result<Box<dyn Initializer>> where Self: Sized {
        let init_provider = crate::config::CONFIG.read()
            .await.get("INIT_PROVIDER")?;
        match init_provider {
            "flox" => Ok(Box::new(FloxInitializer)),
            "rust" => Ok(Box::new(RustNativeInitializer::with_command_git())),
            _ => Ok(Box::new(FloxInitializer))
        }
    }
    async fn init(&self, package_name: &str, builder: &FloxBuilder) -> Result<InitResult>;  
    fn cleanup() -> Result<()> where Self: Sized {

        std::fs::remove_dir_all("./pkgs")?;
        std::fs::remove_file("./flake.nix")?;

        Ok(())
    }
}

struct FloxInitializer;

struct RustNativeInitializer {
    git_provider: Box<dyn GitProvider + Send + Sync>
}

impl RustNativeInitializer {
    fn with_command_git() -> Self {
        return RustNativeInitializer {
            git_provider: Box::new(GitCommandProvider)
        }
    }
}

#[async_trait]
impl Initializer for FloxInitializer {
   
    async fn init(&self, package_name: &str, builder: &FloxBuilder) -> Result<InitResult> {
        let output = CommandRunner::run_in_flox("init", 
            &vec!["--template",&format!("{}", builder), "--name", package_name]).await?;

        Ok(InitResult {message: output})
    }
}

/// 
#[async_trait]
impl Initializer for RustNativeInitializer {
    /// Initialize a flox project
    /// This directly uses nix instead of Flox because the flox shell script currently uses a 
    /// input system, so this is a faithful adoption of the command using the nix command directly.
    /// Because this is meant to be a sdk method, the builder is passed in, but the implementor can use
    /// Other(String) to call any template.
    /// 
    /// This will also create a git repository if it doesn't exist.
    async fn init(&self, package_name: &str, builder: &FloxBuilder) -> Result<InitResult> {
        info!("init {}, {}", package_name, builder);
                    

        if !Path::new("flox.nix").exists() {
            // Init with _init if we haven't already.
            info!("No flox.nix exists, running flox#templates._init");

            let run = CommandRunner::run_in_nix("flake", &vec!["init","--template","flox#templates._init"])
                .await ;

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
         
        match CommandRunner::run_in_nix("flake",
            &vec!["init","--template", &format!("flox#templates.{}", builder)]).await {            
                Ok(response) => info!("Ran flox builder template. {}", response),
                Err(e) => {
                    error!("FXXXX: Error initializing flox: {}",e);
                    // fatal, 
                    return Err(e);
                }
        };        
        // TODO move this to a nix runner
        let output = Command::new(get_nix_cmd())
                .envs(&build_flox_env()?)
                .arg("flake")
                .arg("init")
                .arg("--template")            
                .arg(format!("flox#templates.{}", builder))
            .output().await?;

            let nix_response = std::str::from_utf8(&output.stdout)?;
            let nix_err_response = std::str::from_utf8(&output.stderr)?;

            info!("flake init out: {} err:{}", nix_response, nix_err_response);

        // after init we create some structure
        std::fs::create_dir_all(format!("pkgs/{}", package_name))?;
        // move the default.nix into the pkgs directory
        self.git_provider.mv(Path::new("pkgs/default.nix"), 
            Path::new(&format!("pkgs/{}/default.nix", package_name))).await?;

        Ok(InitResult::new("Done"))
    }
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use anyhow::Result;

    use super::{FloxInitializer, Initializer, RustNativeInitializer};

    #[tokio::test]
    async fn test_init_cmd() -> Result<()> {
        std::env::set_var("RUST_LOG", "info");

        pretty_env_logger::init();

        let builder = crate::models::FloxBuilder::RustPackage;

        let flox_init = FloxInitializer;

        // initialize a rust native initializer (which uses nix via a Command) using the command git 
        let flox_native_init = RustNativeInitializer::with_command_git();

        flox_native_init.init("test_pkg", &builder).await?;
        
        RustNativeInitializer::cleanup()?;

        flox_init.init("test_pkg", &builder).await?;

        FloxInitializer::cleanup()?;

        // ensure cleanup 
        assert_eq!(Path::new("./flake.nix").exists(), false);
        assert_eq!(Path::new("./pkgs").exists(), false);

        Ok(())
    }
}
