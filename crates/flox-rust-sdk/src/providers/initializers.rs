use log::{error, info};
use runix::{arguments::NixArgs, command::FlakeInit, NixBackend, Run};
use std::path::Path;

use crate::flox::{Flox, FloxNixApi};

use thiserror::Error;

use super::git::GitProvider;

pub struct Initializer<'flox> {
    flox: &'flox Flox,
    template_name: String,
}

#[derive(Error, Debug)]
pub enum InitError<Nix: NixBackend, Git: GitProvider>
where
    FlakeInit: Run<Nix>,
{
    #[error("Error initializing git repo: {0}")]
    InitRepo(Git::InitError),
    #[error("Error initializing base template with Nix")]
    NixInitBase(<FlakeInit as Run<Nix>>::Error),
    #[error("Error initializing template with Nix")]
    NixInit(<FlakeInit as Run<Nix>>::Error),
}

#[derive(Error, Debug)]
pub enum CleanupInitializerError {
    #[error("Error removing pkgs")]
    RemovePkgs(std::io::Error),
    #[error("Error removing flake.nix")]
    RemoveFlake(std::io::Error),
}
impl Initializer<'_> {
    pub async fn init<Nix: FloxNixApi, Git: GitProvider>(&self) -> Result<(), InitError<Nix, Git>>
    where
        FlakeInit: Run<Nix>,
    {
        let nix = self.flox.nix::<Nix>();

        if !Path::new("flox.nix").exists() {
            // Init with _init if we haven't already.
            info!("No flox.nix exists, running flox#templates._init");

            FlakeInit {
                template: Some("flox#templates._init".to_string().into()),
                ..Default::default()
            }
            .run(&nix, &NixArgs::default())
            .await
            .map_err(InitError::NixInitBase)?;
        }

        // create a git repo at this spot
        if !Path::new(".git").exists() {
            info!("No git repository locally, creating one");
            self.flox
                .git_provider::<Git>()
                .init_repo()
                .await
                .map_err(InitError::InitRepo)?;
        }

        FlakeInit {
            template: Some(format!("flox#templates.{}", self.template_name).into()),
            ..Default::default()
        }
        .run(&nix, &NixArgs::default())
        .await
        .map_err(InitError::NixInit)?;

        Ok(())
    }

    pub async fn cleanup<Git: GitProvider>(&self) -> Result<(), CleanupInitializerError> {
        std::fs::remove_dir_all("./pkgs").map_err(CleanupInitializerError::RemovePkgs)?;
        std::fs::remove_file("./flake.nix").map_err(CleanupInitializerError::RemoveFlake)?;

        Ok(())
    }
}
