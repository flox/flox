use fs_extra;
use nix_editor;
use runix::NixBackend;
use runix::{
    arguments::{eval::EvaluationArgs, NixArgs},
    command::Build,
    installable::Installable,
    Run,
};
use std::fs;
use std::io;
use std::path::PathBuf;
use tempfile;
use thiserror::Error;

use crate::{
    flox::{Flox, FloxNixApi},
    prelude::flox_package::FloxPackage,
    utils::errors::IoError,
};

static FLOX_NIX: &str = "flox.nix";
static CATALOG_JSON: &str = "catalog.json";

pub struct Environment<'flox> {
    flox: &'flox Flox,
    // The toplevel directory for the flake containing this environment
    flake_dir: PathBuf,
    /// The directory within a flake containing this environment, e.g. pkgs/my-package. This is a
    /// relative, not absolute path
    subdir: PathBuf,
    attr_path: String,
    flox_nix: PathBuf,
    catalog_json: PathBuf,
}

struct BuiltEnvironment {
    result: PathBuf,
}

/////////
// Errors
/////////
#[derive(Error, Debug)]
pub enum EnvironmentError {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error("Failed to write modifications to {} file: {0}", FLOX_NIX)]
    ModifyFloxNix(nix_editor::write::WriteError),
    #[error("Environment directory should be a subdirectory of a flake, e.g. `pkgs/my-pkg`, but it was {dir}")]
    TooShortDirectory { dir: PathBuf },
    #[error("floxEnv directory cannot end in ..")]
    DotDot,
    #[error("Couldn't copy {dir}: {err}")]
    CopyDir {
        dir: PathBuf,
        err: fs_extra::error::Error,
    },
}

#[derive(Error, Debug)]
pub enum EnvironmentListError<Nix: NixBackend>
where
    Build: Run<Nix>,
{
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    #[error(transparent)]
    Build(<Build as Run<Nix>>::Error),
}

#[derive(Error, Debug)]
pub enum EnvironmentEditError<Nix: NixBackend>
where
    Build: Run<Nix>,
{
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    #[error(transparent)]
    Build(<Build as Run<Nix>>::Error),
}

#[derive(Error, Debug)]
pub enum EnvironmentInstallError<Nix: NixBackend>
where
    Build: Run<Nix>,
{
    #[error(transparent)]
    Environment(#[from] EnvironmentError),

    #[error(transparent)]
    Build(#[from] EnvironmentBuildError<Nix>),
}

#[derive(Error, Debug)]
pub enum EnvironmentRemoveError<Nix: NixBackend>
where
    Build: Run<Nix>,
{
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    #[error(transparent)]
    Build(<Build as Run<Nix>>::Error),
}

#[derive(Error, Debug)]
pub enum EnvironmentBuildError<Nix: NixBackend>
where
    Build: Run<Nix>,
{
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    #[error(transparent)]
    Build(<Build as Run<Nix>>::Error),
}

///////////////////
// impl Environment
///////////////////
impl<'flox> Environment<'flox> {
    pub fn new(flox: &'flox Flox, dir: PathBuf) -> Result<Self, EnvironmentError> {
        let absolute = dir.canonicalize().map_err(|err| IoError::Canonicalize {
            dir: dir.clone(),
            err,
        })?;
        let ancestors = absolute.ancestors();
        match ancestors.collect::<Vec<_>>()[..] {
            // TODO this doesn't support nested packages
            // e.g. my-pkg, pkg, repo name
            [env_path, parent_path, flake_dir, ..] => {
                let parent_name = parent_path.file_name().ok_or(EnvironmentError::DotDot)?;
                let env_name = env_path.file_name().ok_or(EnvironmentError::DotDot)?;
                Ok(Environment {
                    flox,
                    flake_dir: flake_dir.to_path_buf(),
                    subdir: PathBuf::from(parent_name).join(PathBuf::from(env_name)),
                    attr_path: format!("floxEnvs.{}.{}", flox.system, env_name.to_string_lossy(),),
                    flox_nix: env_path.join(FLOX_NIX),
                    catalog_json: env_path.join(CATALOG_JSON),
                })
            }
            _ => Err(EnvironmentError::TooShortDirectory { dir }),
        }
    }

    pub async fn list<Nix: FloxNixApi>(&self) -> Result<(), EnvironmentListError<Nix>>
    where
        Build: Run<Nix>,
    {
        todo!()
    }

    pub async fn edit<Nix: FloxNixApi>(&self) -> Result<(), EnvironmentEditError<Nix>>
    where
        Build: Run<Nix>,
    {
        todo!()
    }

    pub async fn install<Nix: FloxNixApi>(
        &self,
        packages: &[FloxPackage],
    ) -> Result<(), EnvironmentInstallError<Nix>>
    where
        Build: Run<Nix>,
    {
        let original_file_contents = self.read_flox_nix().await?;

        let (edited, n_new) = packages.iter().try_fold(
            (original_file_contents, 0),
            |(flox_nix_contents, n_installed),
             package|
             -> Result<(String, i32), EnvironmentError> {
                // reference to packages.<package>
                let query = format!("packages.{}", package);

                let new_content = nix_editor::write::write(&flox_nix_contents, &query, "{}")
                    .map_err(EnvironmentError::ModifyFloxNix)?;
                Ok((new_content, n_installed + 1))
            },
        )?;

        if n_new > 0 {
            let built_environment = self.build(&edited).await?;
            self.write_environment(&edited, &built_environment)?;
        }

        match n_new {
            // TODO 0 is unreachable, but I'm leaving this in so we can more easily add
            // https://github.com/flox/flox-rust-sdk/pull/61
            0 => warn!("no new packages installed"),
            1 => info!("{n_new} new package installed"),
            _ => info!("{n_new} new packages installed"),
        }
        Ok(())
    }

    pub async fn remove<Nix: FloxNixApi>(
        &self,
        package: FloxPackage,
    ) -> Result<(), EnvironmentRemoveError<Nix>>
    where
        Build: Run<Nix>,
    {
        todo!()
    }

    /////////////////
    // Helper methods
    /////////////////
    async fn read_flox_nix(&self) -> Result<String, EnvironmentError> {
        let file_contents = tokio::fs::read_to_string(&self.flox_nix)
            .await
            .map_err(|err| IoError::Open {
                file: self.flox_nix.clone(),
                err,
            })?;
        Ok(file_contents)
    }

    async fn write_temp_environment(
        &self,
        new_flox_nix: &str,
    ) -> Result<PathBuf, EnvironmentError> {
        let temp_dir = tempfile::Builder::new()
            .prefix("environment-build")
            .tempdir_in(&self.flox.temp_dir)
            .map_err(|err| IoError::CreateTempDir {
                dir: self.flox.temp_dir.clone(),
                err,
            })?
            .into_path();

        // If at some point we know exactly what files we need, we could
        // avoid copying the whole directory
        fs_extra::dir::copy(
            &self.flake_dir,
            &temp_dir,
            &fs_extra::dir::CopyOptions::new(),
        )
        .map_err(|err| EnvironmentError::CopyDir {
            dir: self.flake_dir.clone(),
            err,
        })?;
        let temp_flake_dir =
            temp_dir.join(self.flake_dir.file_name().ok_or(EnvironmentError::DotDot)?);
        let temp_flox_nix = temp_flake_dir.join(&self.subdir).join(FLOX_NIX);
        fs::write(&temp_flox_nix, new_flox_nix).map_err(|err| IoError::Write {
            file: temp_flox_nix.clone(),
            err,
        })?;
        Ok(temp_flake_dir)
    }

    /// Copy an environment to a temporary directory, write edits to flox.nix, and attempt a build
    ///
    /// The temporary directory will remain on disk until the [Flox] instance is dropped (which in
    /// turn cleans up `flox.temp_dir`)
    async fn build<Nix: FloxNixApi>(
        &self,
        new_flox_nix: &str,
    ) -> Result<BuiltEnvironment, EnvironmentBuildError<Nix>>
    where
        Build: Run<Nix>,
    {
        let temp_flake_dir = self.write_temp_environment(new_flox_nix).await?;

        let nix = self.flox.nix(Vec::new());

        let nix_args = NixArgs::default();

        let temp_installable = Installable::new(
            temp_flake_dir.to_string_lossy().to_string(),
            self.attr_path.clone(),
        );
        let command = Build {
            installables: [temp_installable].into(),
            eval: EvaluationArgs {
                impure: true.into(),
            },
            ..Default::default()
        };

        command
            .run(&nix, &nix_args)
            .await
            .map_err(EnvironmentBuildError::Build)?;
        // TODO as far as I can tell the above never fails
        Ok(BuiltEnvironment {
            // TODO use --out-link
            result: PathBuf::from("./result"),
        })
    }

    fn write_environment(
        &self,
        new_flox_nix: &str,
        built_environment: &BuiltEnvironment,
    ) -> Result<(), EnvironmentError> {
        // environments potentially update their catalog in the process of a build because unlocked
        // packages (e.g. nixpkgs-flox.hello) must be pinned to a specific version which is added to
        // the catalog
        let result_catalog_json = built_environment.result.join(CATALOG_JSON);
        copy_file_without_permissions(&result_catalog_json, &self.catalog_json)?;
        fs::write(&self.flox_nix, new_flox_nix).map_err(|err| IoError::Write {
            file: self.flox_nix.clone(),
            err,
        })?;

        Ok(())
    }
}

///////////////////
// Helper functions
///////////////////

/// Using fs::copy copies permissions from the Nix store, which we don't want, so open (or
/// create) the files and copy with io::copy
fn copy_file_without_permissions(from: &PathBuf, to: &PathBuf) -> Result<(), EnvironmentError> {
    let mut to_file = fs::File::options()
        .write(true)
        .truncate(true)
        .create(true)
        .open(&to)
        .map_err(|io_err| IoError::Open {
            file: to.to_path_buf(),
            err: io_err,
        })?;
    let mut from_file = fs::File::open(&from).map_err(|io_err| IoError::Open {
        file: from.to_path_buf(),
        err: io_err,
    })?;
    io::copy(&mut from_file, &mut to_file).map_err(|io_err| IoError::Copy {
        file: from.to_path_buf(),
        err: io_err,
    })?;
    Ok(())
}
