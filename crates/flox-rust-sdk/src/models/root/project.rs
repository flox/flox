use std::path::Path;

use derive_more::Constructor;
use log::{debug, info};
use once_cell::sync::Lazy;
use regex::Regex;
use runix::arguments::NixArgs;
use runix::command::FlakeInit;
use runix::installable::Installable;
use runix::{NixBackend, Run};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{Closed, Root, RootGuard};
use crate::flox::FloxNixApi;
use crate::providers::git::GitProvider;
use crate::utils::guard::Guard;
use crate::utils::{find_and_replace, FindAndReplaceError};

static PNAME_DECLARATION: Lazy<Regex> = Lazy::new(|| Regex::new(r#"pname = ".*""#).unwrap());
static PACKAGE_NAME_PLACEHOLDER: &str = "__PACKAGE_NAME__";

#[derive(Constructor, Debug)]
pub struct Project<T> {
    pub inner: T,
}

impl<'flox, Git: GitProvider> Root<'flox, Closed<Git>> {
    /// Guards opening a project
    ///
    /// - Resolves as initialized if a `flake.nix` is present
    /// - Resolves as uninitialized if not
    pub async fn guard(
        self,
    ) -> Result<RootGuard<'flox, Project<Git>, Closed<Git>>, OpenProjectError> {
        let repo = &self.state.inner;

        let root = repo.workdir().ok_or(OpenProjectError::WorkdirNotFound)?;

        if root.join("flake.nix").exists() {
            Ok(Guard::Initialized(Root {
                flox: self.flox,
                state: Project::new(self.state.inner),
            }))
        } else {
            Ok(Guard::Uninitialized(self))
        }
    }
}

/// Implementation to upgrade into an open Project
impl<'flox, Git: GitProvider> RootGuard<'flox, Project<Git>, Closed<Git>> {
    /// Initialize a new project in the workdir of a git root or return
    /// an existing project if it exists.
    pub async fn init_project<Nix: FloxNixApi>(
        self,
        nix_extra_args: Vec<String>,
    ) -> Result<Root<'flox, Project<Git>>, InitProjectError<Nix, Git>>
    where
        FlakeInit: Run<Nix>,
    {
        if let Guard::Initialized(i) = self {
            return Ok(i);
        }

        let uninit = match self {
            Guard::Uninitialized(u) => u,
            _ => unreachable!(), // returned above
        };

        let repo = uninit.state.inner;

        let root = repo
            .workdir()
            .ok_or(InitProjectError::<Nix, Git>::WorkdirNotFound)?;

        let nix = uninit.flox.nix(nix_extra_args);

        FlakeInit {
            template: Some("flox#templates._init".to_string().into()),
            ..Default::default()
        }
        .run(&nix, &NixArgs::default())
        .await
        .map_err(InitProjectError::NixInitBase)?;

        repo.add(&[&root.join("flake.nix")])
            .await
            .map_err(InitProjectError::GitAdd)?;

        Ok(Root {
            flox: uninit.flox,
            state: Project::new(repo),
        })
    }
}

/// Implementations for an opened project
impl<Git: GitProvider> Root<'_, Project<Git>> {
    /// Get the root directory of the project flake
    ///
    /// currently the git root but may be a subdir with a flake.nix
    pub fn workdir(&self) -> Option<&Path> {
        self.state.inner.workdir()
    }

    /// Path to the `.git` directory
    pub fn path(&self) -> &Path {
        self.state.inner.path()
    }

    /// Add a new flox style package from a template.
    /// Uses `nix flake init` to retrieve files
    /// and postprocesses the generic templates.
    pub async fn init_flox_package<Nix: FloxNixApi>(
        &self,
        nix_extra_args: Vec<String>,
        template: Installable,
        name: &str,
    ) -> Result<(), InitFloxPackageError<Nix, Git>>
    where
        FlakeInit: Run<Nix>,
    {
        let repo = &self.state.inner;

        let nix = self.flox.nix(nix_extra_args);

        let root = repo
            .workdir()
            .ok_or(InitFloxPackageError::WorkdirNotFound)?;

        FlakeInit {
            template: Some(template.to_string().into()),
            ..Default::default()
        }
        .run(&nix, &NixArgs {
            cwd: root.to_path_buf().into(),
            ..NixArgs::default()
        })
        .await
        .map_err(InitFloxPackageError::NixInit)?;

        let old_package_path = root.join("pkgs/default.nix");

        match tokio::fs::File::open(&old_package_path).await {
            // legacy path. Drop after we merge template changes to floxpkgs
            Ok(mut file) => {
                let mut package_contents = String::new();
                file.read_to_string(&mut package_contents)
                    .await
                    .map_err(InitFloxPackageError::ReadTemplateFile)?;

                // Drop handler should clear our file handle in case we want to delete it
                drop(file);

                let new_contents =
                    PNAME_DECLARATION.replace(&package_contents, format!(r#"pname = "{name}""#));

                let new_package_dir = root.join("pkgs").join(name);
                debug!("creating dir: {}", new_package_dir.display());
                tokio::fs::create_dir_all(&new_package_dir)
                    .await
                    .map_err(InitFloxPackageError::MkNamedDir)?;

                let new_package_path = new_package_dir.join("default.nix");

                repo.rm(&[&old_package_path], false, true, false)
                    .await
                    .map_err(InitFloxPackageError::RemoveUnnamedFile)?;

                let mut file = tokio::fs::File::create(&new_package_path)
                    .await
                    .map_err(InitFloxPackageError::OpenNamed)?;

                file.write_all(new_contents.as_bytes())
                    .await
                    .map_err(InitFloxPackageError::WriteTemplateFile)?;

                repo.add(&[&new_package_path])
                    .await
                    .map_err(InitFloxPackageError::GitAdd)?;

                // this might technically be a lie, but it's close enough :)
                info!("renamed: pkgs/default.nix -> pkgs/{name}/default.nix");
            },
            Err(err) => match err.kind() {
                std::io::ErrorKind::NotFound => {
                    let old_proto_pkg_path = root.join("pkgs").join(PACKAGE_NAME_PLACEHOLDER);
                    let new_proto_pkg_path = root.join("pkgs").join(name);

                    repo.mv(&old_proto_pkg_path, &new_proto_pkg_path)
                        .await
                        .map_err(InitFloxPackageError::GitMv)?;
                    info!(
                        "moved: {} -> {}",
                        old_proto_pkg_path.to_string_lossy(),
                        new_proto_pkg_path.to_string_lossy()
                    );

                    // our minimal "templating" - Replace any occurrences of
                    // PACKAGE_NAME_PLACEHOLDER with name
                    find_and_replace(&new_proto_pkg_path, PACKAGE_NAME_PLACEHOLDER, name)
                        .await
                        .map_err(InitFloxPackageError::<Nix, Git>::ReplacePackageName)?;

                    repo.add(&[&new_proto_pkg_path])
                        .await
                        .map_err(InitFloxPackageError::GitAdd)?;
                },
                _ => return Err(InitFloxPackageError::OpenTemplateFile(err)),
            },
        };
        Ok(())
    }

    /// Delete flox files from repo
    pub async fn cleanup_flox(self) -> Result<(), CleanupInitializerError> {
        tokio::fs::remove_dir_all("./pkgs")
            .await
            .map_err(CleanupInitializerError::RemovePkgs)?;
        tokio::fs::remove_file("./flake.nix")
            .await
            .map_err(CleanupInitializerError::RemoveFlake)?;

        Ok(())
    }
}

/// Errors occuring while trying to upgrade to an [`Open<Git>`] [Root]
#[derive(Error, Debug)]
pub enum OpenProjectError {
    #[error("Could not determine repository root")]
    WorkdirNotFound,
}

#[derive(Error, Debug)]
pub enum InitProjectError<Nix: NixBackend, Git: GitProvider>
where
    FlakeInit: Run<Nix>,
{
    #[error("Could not determine repository root")]
    WorkdirNotFound,

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
pub enum InitFloxPackageError<Nix: NixBackend, Git: GitProvider>
where
    FlakeInit: Run<Nix>,
{
    #[error("Could not determine repository root")]
    WorkdirNotFound,
    #[error("Error initializing template with Nix")]
    NixInit(<FlakeInit as Run<Nix>>::Error),
    #[error("Error moving template file to named location using Git")]
    MvNamed(Git::MvError),
    #[error("Error opening template file")]
    OpenTemplateFile(std::io::Error),
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
    #[error("Error removing old unnamed file using Git")]
    RemoveUnnamedFile(Git::RmError),
    #[error("Error staging new renamed file in Git")]
    GitAdd(Git::AddError),
    #[error("Error moving file in Git")]
    GitMv(Git::MvError),
    #[error("Error replacing {}: {0}", PACKAGE_NAME_PLACEHOLDER)]
    ReplacePackageName(FindAndReplaceError),
}

#[derive(Error, Debug)]
pub enum CleanupInitializerError {
    #[error("Error removing pkgs")]
    RemovePkgs(std::io::Error),
    #[error("Error removing flake.nix")]
    RemoveFlake(std::io::Error),
}
