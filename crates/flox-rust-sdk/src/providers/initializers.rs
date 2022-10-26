//! Attempt to implement some parts of the bash script in rust
//!
//!
//!
//!
use log::{error, info};
use std::path::Path;

use crate::{models::*, utils::runner::CommandRunner};
use anyhow::Result;
use async_trait::async_trait;
use tokio::process::Command;

use super::git::{GitCommandProvider, GitProvider};
use crate::environment::*;

pub async fn get_provider() -> Result<Box<dyn Initializer>> {
    let init_provider: String = crate::config::CONFIG
        .read()
        .await
        .get("init_provider")
        .unwrap_or_else(|_r| String::from("flox"));

    Ok(match init_provider.as_str() {
        "flox" => Box::new(FloxInitializer),
        "rust" => Box::new(RustNativeInitializer::with_command_git()),
        _ => Box::new(FloxInitializer),
    })
}

#[async_trait]
pub trait Initializer {
    fn name(&self) -> String;
    async fn init(&self, package_name: &str, builder: &FloxBuilder) -> Result<InitResult>;

    /// Cleanup the current environment. Removes ./pkgs and ./flake.nix
    /// There probably needs to be more done here, but this is a start.
    fn cleanup() -> Result<()>
    where
        Self: Sized,
    {
        std::fs::remove_dir_all("./pkgs")?;
        std::fs::remove_file("./flake.nix")?;

        Ok(())
    }
}

/// An Initializer that just uses the flox bash CLI
struct FloxInitializer;

/// A native implementation of the

struct RustNativeInitializer {
    git_provider: Box<dyn GitProvider + Send + Sync>,
}

impl RustNativeInitializer {
    fn with_command_git() -> Self {
        RustNativeInitializer {
            git_provider: Box::new(GitCommandProvider),
        }
    }
}

#[async_trait]
impl Initializer for FloxInitializer {
    fn name(&self) -> String {
        String::from("FloxInitializer")
    }
    async fn init(&self, package_name: &str, builder: &FloxBuilder) -> Result<InitResult> {
        let output = CommandRunner::run_in_flox(
            "init",
            &vec![
                "--template",
                &format!("{}", builder),
                "--name",
                package_name,
            ],
        )
        .await?;

        Ok(InitResult { message: output })
    }
}

///
#[async_trait]
impl Initializer for RustNativeInitializer {
    fn name(&self) -> String {
        String::from("RustNativeInitializer")
    }
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

            let run = CommandRunner::run_in_nix(
                "flake",
                &vec!["init", "--template", "flox#templates._init"],
            )
            .await;

            match run {
                Ok(response) => info!("Ran flox initialization template. {}", response),
                Err(e) => error!("FXXXX: Error initializing flox: {}", e),
            };
        }

        // create a git repo at this spot
        if !Path::new(".git").exists() {
            info!("No git repository locally, creating one");
            self.git_provider.init_repo().await?;
        }

        match CommandRunner::run_in_nix(
            "flake",
            &vec!["init", "--template", &format!("flox#templates.{}", builder)],
        )
        .await
        {
            Ok(response) => info!("Ran flox builder template. {}", response),
            Err(e) => {
                error!("FXXXX: Error initializing flox: {}", e);
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
            .output()
            .await?;

        let nix_response = std::str::from_utf8(&output.stdout)?;
        let nix_err_response = std::str::from_utf8(&output.stderr)?;

        info!("flake init out: {} err:{}", nix_response, nix_err_response);

        // after init we create some structure
        std::fs::create_dir_all(format!("pkgs/{}", package_name))?;
        // move the default.nix into the pkgs directory
        self.git_provider
            .mv(
                Path::new("pkgs/default.nix"),
                Path::new(&format!("pkgs/{}/default.nix", package_name)),
            )
            .await?;

        Ok(InitResult::new("Done"))
    }
}

#[cfg(test)]
mod test {
    use std::{env, path::Path};

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

    #[tokio::test]
    async fn test_default_init_di() -> Result<()> {
        let provider = super::get_provider().await?;

        // default
        assert_eq!(provider.name(), "FloxInitializer");

        Ok(())
    }
    #[tokio::test]
    async fn test_rust_init_di() -> Result<()> {
        env::set_var("FLOX_INIT_PROVIDER", "rust");

        // config is lazy, so we'll set the env and then try to grab the provider

        let provider = super::get_provider().await?;

        assert_eq!(provider.name(), "RustNativeInitializer");

        Ok(())
    }

    #[tokio::test]
    async fn test_rust_init_flox() -> Result<()> {
        env::set_var("FLOX_INIT_PROVIDER", "flox");

        // config is lazy, so we'll set the env and then try to grab the provider

        let provider = super::get_provider().await?;

        assert_eq!(provider.name(), "FloxInitializer");

        Ok(())
    }
}
