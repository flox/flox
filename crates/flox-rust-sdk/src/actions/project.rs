use std::path::{Path, PathBuf};

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
use crate::providers::git::{GitDiscoverError, GitProvider};

static PNAME_DECLARATION: Lazy<Regex> = Lazy::new(|| Regex::new(r#"pname = ".*""#).unwrap());

trait Initialize<I> {
    type InitError;
    fn init(self) -> Result<I, Self::InitError>;
}

pub enum Guard<I, U> {
    Initialized(I),
    Uninitialized(U),
}

impl<I, U> Guard<I, U> {
    pub fn open(self) -> Result<I, Self> {
        match self {
            Guard::Initialized(i) => Ok(i),
            Guard::Uninitialized(_) => Err(self),
        }
    }
}

type ProjectGuard<'flox, I, U> = Guard<Project<'flox, I>, Project<'flox, U>>;

#[derive(Constructor, Debug)]
pub struct Open<T> {
    pub inner: T,
}

#[derive(Constructor, Debug)]
pub struct Closed<T> {
    pub inner: T,
}

#[derive(Constructor, Debug)]
pub struct Project<'flox, State> {
    pub flox: &'flox Flox,
    pub state: State,
}

impl<'flox> Project<'flox, Closed<PathBuf>> {
    pub async fn guard<Git: GitProvider>(
        self,
    ) -> Result<ProjectGuard<'flox, Closed<Git>, Closed<PathBuf>>, ProjectDiscoverGitError<Git>>
    {
        match Git::discover(&self.state.inner).await {
            Ok(repo) => Ok(Guard::Initialized(Project {
                flox: self.flox,
                state: Closed::new(repo),
            })),
            Err(err) if err.not_found() => Ok(Guard::Uninitialized(Project {
                flox: self.flox,
                state: Closed::new(self.state.inner),
            })),
            Err(err) => Err(ProjectDiscoverGitError::DiscoverRepoError(err)),
        }
    }
}

impl<'flox, Git: GitProvider> ProjectGuard<'flox, Closed<Git>, Closed<PathBuf>> {
    pub fn workdir(&self) -> Option<&Path> {
        match self {
            Guard::Initialized(i) => i.workdir(),
            Guard::Uninitialized(u) => Some(&u.state.inner),
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Guard::Initialized(i) => i.path(),
            Guard::Uninitialized(u) => &u.state.inner,
        }
    }

    pub async fn init_git(self) -> Result<Project<'flox, Closed<Git>>, ProjectInitGitError<Git>> {
        match self {
            Guard::Initialized(i) => Ok(i),
            Guard::Uninitialized(u) => {
                let repo = Git::init(&u.state.inner)
                    .await
                    .map_err(ProjectInitGitError::InitRepoError)?;

                Ok(Project {
                    flox: u.flox,
                    state: Closed::new(repo),
                })
            },
        }
    }
}

impl<'flox, Git: GitProvider> Project<'flox, Closed<Git>> {
    pub fn workdir(&self) -> Option<&Path> {
        self.state.inner.workdir()
    }

    pub fn path(&self) -> &Path {
        self.state.inner.path()
    }

    /// Guards opening a project
    ///
    /// - Resolves as initilaized if a `flake.nix` is present
    /// - Resolves as unitialized if not
    pub async fn guard(
        self,
    ) -> Result<ProjectGuard<'flox, Open<Git>, Closed<Git>>, OpenProjectError> {
        let repo = self.state.inner;

        let root = repo.workdir().ok_or(OpenProjectError::WorkdirNotFound)?;

        if root.join("flake.nix").exists() {
            Ok(Guard::Initialized(Project {
                flox: self.flox,
                state: Open::new(repo),
            }))
        } else {
            Ok(Guard::Uninitialized(Project {
                flox: self.flox,
                state: Closed::new(repo),
            }))
        }
    }
}

impl<'flox, Git: GitProvider> ProjectGuard<'flox, Open<Git>, Closed<Git>> {
    pub async fn init_project<Nix: FloxNixApi>(
        self,
        nix_extra_args: Vec<String>,
    ) -> Result<Project<'flox, Open<Git>>, InitProjectError<Nix, Git>>
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

        Ok(Project {
            flox: uninit.flox,
            state: Open::new(repo),
        })
    }
}

impl<'flox, T> Project<'flox, Closed<T>> {
    pub fn closed(flox: &'flox Flox, inner: T) -> Self {
        Project {
            flox,
            state: Closed::new(inner),
        }
    }
}

impl<Git: GitProvider> Project<'_, Open<Git>> {
    pub fn workdir(&self) -> Option<&Path> {
        self.state.inner.workdir()
    }

    pub fn path(&self) -> &Path {
        self.state.inner.path()
    }

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

        let mut file = match tokio::fs::File::open(&old_package_path).await {
            Ok(f) => f,
            Err(err) => match err.kind() {
                std::io::ErrorKind::NotFound => {
                    return Ok(());
                },
                _ => return Err(InitFloxPackageError::OpenTemplateFile(err)),
            },
        };

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

        Ok(())
    }

    /// Delete flox files from repo
    pub async fn delete(self) -> Result<(), CleanupInitializerError> {
        tokio::fs::remove_dir_all("./pkgs")
            .await
            .map_err(CleanupInitializerError::RemovePkgs)?;
        tokio::fs::remove_file("./flake.nix")
            .await
            .map_err(CleanupInitializerError::RemoveFlake)?;

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum ProjectDiscoverGitError<Git: GitProvider> {
    #[error("Error attempting to discover repository: {0}")]
    DiscoverRepoError(Git::DiscoverError),
}

#[derive(Error, Debug)]
pub enum ProjectInitGitError<Git: GitProvider> {
    #[error("Error initializing repository: {0}")]
    InitRepoError(Git::InitError),
}

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
}

#[derive(Error, Debug)]
pub enum CleanupInitializerError {
    #[error("Error removing pkgs")]
    RemovePkgs(std::io::Error),
    #[error("Error removing flake.nix")]
    RemoveFlake(std::io::Error),
}
