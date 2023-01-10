use std::borrow::Cow;
use std::path::Path;

use derive_more::Constructor;
use log::{debug, error, info};
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
pub struct Initializer<'flox, 'a> {
    flox: &'flox Flox,
    dir: &'a Path,
    nix_arguments: Vec<String>,
}

#[derive(Error, Debug)]
pub enum InitFloxPackageError<Nix: NixBackend, Git: GitProvider>
where
    FlakeInit: Run<Nix>,
{
    #[error("Error initializing template with Nix")]
    NixInit(<FlakeInit as Run<Nix>>::Error),
    #[error("Error moving template file to named location")]
    MvNamed(std::io::Error),
    #[error("Error moving template file to named location using Git")]
    MvNamedGit(Git::MvError),
    #[error("Error reading template file contents")]
    ReadTemplateFile(std::io::Error),
    #[error("Error truncating template file")]
    TruncateTemplateFile(std::io::Error),
    #[error("Error writing to template file")]
    WriteTemplateFile(std::io::Error),
    #[error("Error making named directory")]
    MkNamedDir(std::io::Error),
    #[error("Error opening new renamed file for writing")]
    OpenNamed(std::io::Error),
    #[error("Error removing old unnamed file")]
    RemoveUnnamedFile(std::io::Error),
    #[error("Error removing old unnamed file using Git")]
    RemoveUnnamedFileGit(Git::RmError),
    #[error("Error staging new renamed file in Git")]
    GitAdd(Git::AddError),
}

#[derive(Error, Debug)]
pub enum EnsureFloxProjectError<Nix: NixBackend, Git: GitProvider>
where
    FlakeInit: Run<Nix>,
{
    #[error("Error initializing base template with Nix")]
    NixInitBase(<FlakeInit as Run<Nix>>::Error),
    #[error("Error reading template file contents")]
    ReadTemplateFile(std::io::Error),
    #[error("Error truncating template file")]
    TruncateTemplateFile(std::io::Error),
    #[error("Error writing to template file")]
    WriteTemplateFile(std::io::Error),
    #[error("Error new template file in Git")]
    GitAdd(Git::AddError),
}

#[derive(Error, Debug)]
pub enum InitGitRepoError<Git: GitProvider> {
    #[error("Error getting current directory: {0}")]
    CurrentDirError(std::io::Error),
    #[error("Error attempting to discover git repo: {0}")]
    OpenRepo(Git::DiscoverError),
    #[error("Error initializing git repo: {0}")]
    InitRepo(Git::InitError),
}

#[derive(Error, Debug)]
pub enum CleanupInitializerError {
    #[error("Error removing pkgs")]
    RemovePkgs(std::io::Error),
    #[error("Error removing flake.nix")]
    RemoveFlake(std::io::Error),
}
impl Initializer<'_, '_> {
    pub async fn ensure_flox_project<Nix: FloxNixApi, Git: GitProvider>(
        &self,
        git: &Option<Git>,
    ) -> Result<(), EnsureFloxProjectError<Nix, Git>>
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
            .map_err(EnsureFloxProjectError::NixInitBase)?;

            if let Some(git) = git {
                git.add(&[Path::new("flake.nix")])
                    .await
                    .map_err(EnsureFloxProjectError::GitAdd)?;
            }
        }

        Ok(())
    }

    pub async fn init_flox_package<Nix: FloxNixApi, Git: GitProvider>(
        &self,
        git: &Option<Git>,
        template: Installable,
        name: &str,
    ) -> Result<(), InitFloxPackageError<Nix, Git>>
    where
        FlakeInit: Run<Nix>,
    {
        let nix = self.flox.nix(self.nix_arguments.clone());

        FlakeInit {
            template: Some(template.to_string().into()),
            ..Default::default()
        }
        .run(&nix, &NixArgs::default())
        .await
        .map_err(InitFloxPackageError::NixInit)?;

        let old_package_path = self.dir.join("pkgs/default.nix");

        // TODO do we want to care about some errors?
        if let Ok(mut file) = tokio::fs::File::open(&old_package_path).await {
            let mut package_contents = String::new();
            file.read_to_string(&mut package_contents)
                .await
                .map_err(InitFloxPackageError::ReadTemplateFile)?;

            // Drop handler should clear our file handle in case we want to delete it
            drop(file);

            let new_contents =
                PNAME_DECLARATION.replace(&package_contents, format!(r#"pname = "{name}""#));

            let new_package_dir = self.dir.join("pkgs").join(name);
            debug!("creating dir: {}", new_package_dir.display());
            tokio::fs::create_dir_all(&new_package_dir)
                .await
                .map_err(InitFloxPackageError::MkNamedDir)?;

            let new_package_path = new_package_dir.join("default.nix");

            if let Cow::Owned(s) = new_contents {
                if let Some(git) = git {
                    git.rm(&[&old_package_path], false, true, false)
                        .await
                        .map_err(InitFloxPackageError::RemoveUnnamedFileGit)?;
                } else {
                    tokio::fs::remove_file(&old_package_path)
                        .await
                        .map_err(InitFloxPackageError::RemoveUnnamedFile)?;
                }

                let mut file = tokio::fs::File::create(&new_package_path)
                    .await
                    .map_err(InitFloxPackageError::OpenNamed)?;

                file.write_all(s.as_bytes())
                    .await
                    .map_err(InitFloxPackageError::WriteTemplateFile)?;

                if let Some(git) = git {
                    git.add(&[&new_package_path])
                        .await
                        .map_err(InitFloxPackageError::GitAdd)?;
                }
            } else if let Some(git) = git {
                git.mv(&old_package_path, &new_package_path)
                    .await
                    .map_err(InitFloxPackageError::MvNamedGit)?;
            } else {
                tokio::fs::rename(&old_package_path, &new_package_path)
                    .await
                    .map_err(InitFloxPackageError::MkNamedDir)?;
            }

            // this might technically be a lie, but it's close enough :)
            info!("renamed: pkgs/default.nix -> pkgs/{name}/default.nix");
        }

        Ok(())
    }

    pub async fn cleanup<Git: GitProvider>(&self) -> Result<(), CleanupInitializerError> {
        std::fs::remove_dir_all("./pkgs").map_err(CleanupInitializerError::RemovePkgs)?;
        std::fs::remove_file("./flake.nix").map_err(CleanupInitializerError::RemoveFlake)?;

        Ok(())
    }
}
