use std::borrow::Cow;
use std::path::{Path, PathBuf};

use derive_more::Constructor;
use log::{error, info};
use once_cell::sync::Lazy;
use regex::Regex;
use runix::arguments::NixArgs;
use runix::command::FlakeInit;
use runix::installable::Installable;
use runix::{NixBackend, Run};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::flox::{Flox, FloxNixApi};
use crate::providers::git::GitProvider;

static PNAME_DECLARATION: Lazy<Regex> = Lazy::new(|| Regex::new(r#"pname = ".*""#).unwrap());

#[derive(Constructor)]
pub struct Initializer<'flox> {
    flox: &'flox Flox,
    template: Installable,
    name: String,
    nix_arguments: Vec<String>,
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
    #[error("Error moving template file to named location")]
    MvNamed(Git::MvError),
    #[error("Error reading template file contents")]
    ReadTemplateFile(std::io::Error),
    #[error("Error truncating template file")]
    TruncateTemplateFile(std::io::Error),
    #[error("Error writing to template file")]
    WriteTemplateFile(std::io::Error),
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
        let nix = self.flox.nix(self.nix_arguments.clone());

        if !Path::new("flake.nix").exists() {
            // Init with _init if we haven't already.
            info!("No flake.nix exists, running flox#templates._init");

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
            template: Some(self.template.to_string().into()),
            ..Default::default()
        }
        .run(&nix, &NixArgs::default())
        .await
        .map_err(InitError::NixInit)?;

        // TODO do we want to care about some errors?
        if let Ok(mut file) = tokio::fs::OpenOptions::new()
            .write(true)
            .read(true)
            .create(false)
            .open("pkgs/default.nix")
            .await
        {
            let package_path = ["pkgs", &self.name, "default.nix"]
                .iter()
                .collect::<PathBuf>();
            self.flox
                .git_provider::<Git>()
                .mv(Path::new("pkgs/default.nix"), &package_path)
                .await
                .map_err(InitError::MvNamed)?;

            info!(
                "renamed: pkgs/default.nix -> pkgs/{}/default.nix",
                self.name
            );

            let mut package_contents = String::new();
            file.read_to_string(&mut package_contents)
                .await
                .map_err(InitError::ReadTemplateFile)?;

            let new_contents =
                PNAME_DECLARATION.replace(&package_contents, format!(r#"pname = "{}""#, self.name));

            if let Cow::Owned(s) = new_contents {
                file.set_len(0)
                    .await
                    .map_err(InitError::TruncateTemplateFile)?;
                file.write_all(s.as_bytes())
                    .await
                    .map_err(InitError::WriteTemplateFile)?;
            }
        }

        Ok(())
    }

    pub async fn cleanup<Git: GitProvider>(&self) -> Result<(), CleanupInitializerError> {
        std::fs::remove_dir_all("./pkgs").map_err(CleanupInitializerError::RemovePkgs)?;
        std::fs::remove_file("./flake.nix").map_err(CleanupInitializerError::RemoveFlake)?;

        Ok(())
    }
}
