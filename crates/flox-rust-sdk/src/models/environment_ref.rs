use std::io::{self, ErrorKind};
use std::path::{PathBuf, StripPrefixError};

use log::debug;
use runix::installable::Installable;
use thiserror::Error;

use crate::actions::project;
use crate::flox::Flox;
use crate::providers::git::GitProvider;

static DEFAULT_NAME: &str = "default";
static DEFAULT_OWNER: &str = "local";

#[derive(Debug)]
pub struct Project<'flox, Git: GitProvider> {
    pub flox: &'flox Flox,
    pub project: project::Project<'flox, project::Open<Git>>,
    pub name: PathBuf,
}

#[derive(Error, Debug)]
pub enum FindProjectError {
    #[error("Error checking whether environment is a path: {0}")]
    TryExists(io::Error),
    #[error("Path for environment not found")]
    Missing,
    #[error("Error trying to discover Git repo")]
    DiscoverError,
    #[error("Environment specified exists, but it is not in a Git repo")]
    NotInGitRepo,
    #[error("Error checking for project")]
    OpenError,
    #[error("Found Git repo, but it is not a project")]
    NotProject,
    // TODO be more informative about (or handle) symlinks or fs boundaries or something?
    #[error(
        "Environment is a part of Git repo, but the workdir is not a prefix of environment: {0}"
    )]
    StripPrefix(StripPrefixError),
    #[error("Environment is in a Git repo, but it is bare")]
    BareRepo,
}

impl<'flox, Git: GitProvider> Project<'flox, Git> {
    pub async fn find(
        flox: &'flox Flox,
        environment_path_str: &str,
    ) -> Result<Project<'flox, Git>, FindProjectError> {
        let environment_path = match PathBuf::from(environment_path_str).canonicalize() {
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => return Err(FindProjectError::Missing),
                _ => return Err(FindProjectError::TryExists(err)),
            },
            Ok(x) => x,
        };

        let git_repo = flox
            .project(environment_path.clone())
            .guard::<Git>()
            .await
            .map_err(|_| FindProjectError::DiscoverError)?
            .open()
            .await
            .map_err(|_| FindProjectError::NotInGitRepo)?;

        let project = git_repo
            .guard()
            .await
            .map_err(|_| FindProjectError::OpenError)?
            .open()
            .await
            .map_err(|_| FindProjectError::NotProject)?;

        let workdir = project.workdir().ok_or(FindProjectError::BareRepo)?;
        let name = environment_path
            .strip_prefix(workdir)
            .map_err(FindProjectError::StripPrefix)?
            .to_owned();

        Ok(Project {
            flox,
            project,
            name,
        })
    }

    fn get_installable(&self, system: &str) -> Installable {
        Installable {
            flakeref: format!(
                "git+file://{project_dir}",
                project_dir = self.project.path().display()
            ),
            attr_path: format!("floxEnvs.{system}.{name}", name = self.name.display()),
        }
    }
}

#[derive(Debug)]
pub struct Named<Git: GitProvider> {
    pub owner: String,
    pub name: String,
    pub git: Git,
}

#[derive(Error, Debug)]
pub enum NamedGetCurrentGenError<Git: GitProvider> {
    #[error("Error printing metadata: {0}")]
    Show(Git::ShowError),
    #[error("Metadata file is not valid unicode")]
    MetadataEncoding,
    #[error("Error parsing current generation from metadata: {0}")]
    Parse(#[from] serde_json::Error),
}

#[derive(Error, Debug)]
pub enum FindDefaultOwnerError {
    #[error("Symlink is invalid")]
    DefaultOwnerSymlinkTarget,
    #[error("Error checking symlink")]
    ReadLink(io::Error),
    #[error("Symlink is not valid unicode")]
    DefaultOwnerSymlinkEncoding,
}

#[derive(Error, Debug)]
pub enum FindNamedError<Git: GitProvider> {
    #[error("Error finding default environment owner: {0}")]
    DefaultOwnerSymlinkTarget(#[from] FindDefaultOwnerError),
    #[error("Owner directory is missing")]
    GitDiscoverError(Git::DiscoverError),
}

impl<Git: GitProvider> Named<Git> {
    pub async fn find(flox: &Flox, environment: &str) -> Result<Self, FindNamedError<Git>> {
        let (owner, name) = match environment.rsplit_once('/') {
            None => {
                let default_owner = Self::find_default_owner(flox).await?;

                (
                    default_owner,
                    if environment.is_empty() {
                        DEFAULT_NAME.to_string()
                    } else {
                        environment.to_string()
                    },
                )
            },
            Some((owner, "")) => (owner.to_string(), DEFAULT_NAME.to_string()),
            Some((owner, name)) => (owner.to_string(), name.to_string()),
        };

        let git = Git::discover(Self::meta_dir(flox).join(&owner))
            .await
            .map_err(FindNamedError::GitDiscoverError)?;

        Ok(Named { owner, name, git })
    }

    fn meta_dir(flox: &Flox) -> PathBuf {
        flox.cache_dir.join("meta")
    }

    async fn find_default_owner(flox: &Flox) -> Result<String, FindDefaultOwnerError> {
        let link_path = Self::meta_dir(flox).join(DEFAULT_OWNER);
        debug!(
            "Checking `local` symlink (`{}`) for true name of default user",
            link_path.display()
        );

        match tokio::fs::read_link(link_path).await {
            Ok(p) => Ok(p
                .file_name()
                .ok_or(FindDefaultOwnerError::DefaultOwnerSymlinkTarget)?
                .to_str()
                .ok_or(FindDefaultOwnerError::DefaultOwnerSymlinkEncoding)?
                .to_owned()),
            Err(err) => match err.kind() {
                // `InvalidInput` occurs if the path is not a symlink
                // return DEFAULT_OWNER if it is a directory or doesn't already exist
                ErrorKind::NotFound | ErrorKind::InvalidInput => Ok(DEFAULT_OWNER.to_owned()),
                _ => Err(FindDefaultOwnerError::ReadLink(err)),
            },
        }
    }

    fn get_installable(&self, flox: &Flox, system: &str, gen: &str) -> Installable {
        Installable {
            flakeref: format!(
                "git+file://{meta_dir}/{owner}?ref={system}.{name}&dir={gen}",
                name = self.name,
                owner = self.owner,
                meta_dir = Self::meta_dir(flox).display(),
            ),
            // The git branch varies but the name always remains `default`,
            // which comes from the template
            // https://github.com/flox/flox-bash-private/tree/main/lib/templateFloxEnv/pkgs/default
            // and does not get renamed.
            attr_path: format!(".floxEnvs.{system}.default"),
        }
    }

    async fn get_current_gen(&self, system: &str) -> Result<String, NamedGetCurrentGenError<Git>> {
        let out_os_str = self
            .git
            .show(&format!("{system}.{name}:metadata.json", name = self.name))
            .await
            .map_err(NamedGetCurrentGenError::Show)?;

        let out_str = out_os_str
            .to_str()
            .ok_or(NamedGetCurrentGenError::MetadataEncoding)?;

        Ok(serde_json::from_str(out_str)?)
    }
}

#[derive(Debug)]
pub enum EnvironmentRef<'flox, Git: GitProvider> {
    Named(Named<Git>),
    Project(Project<'flox, Git>),
}

#[derive(Error, Debug)]
pub enum EnvironmentRefError<Git: GitProvider> {
    #[error(transparent)]
    Project(FindProjectError),
    #[error(transparent)]
    Named(FindNamedError<Git>),
}

#[allow(unused)]
impl<Git: GitProvider> EnvironmentRef<'_, Git> {
    /// Try to find a project environment matching the inputted name,
    /// if the dir is missing or is not a Git repo, then try as a named environment
    pub async fn find<'flox>(
        flox: &'flox Flox,
        environment_name: &str,
    ) -> Result<EnvironmentRef<'flox, Git>, EnvironmentRefError<Git>> {
        match Project::find(flox, environment_name).await {
            Ok(p) => Ok(EnvironmentRef::Project(p)),
            Err(FindProjectError::Missing | FindProjectError::NotInGitRepo) => {
                Ok(EnvironmentRef::Named(
                    Named::find(flox, environment_name)
                        .await
                        .map_err(EnvironmentRefError::Named)?,
                ))
            },
            Err(err) => Err(EnvironmentRefError::Project(err)),
        }
    }

    pub async fn get_latest_installable(
        &self,
        flox: &Flox,
    ) -> Result<Installable, NamedGetCurrentGenError<Git>> {
        debug!("Resolving env to installable: {:?}", self);

        let system = &flox.system;

        match self {
            EnvironmentRef::Project(project_ref) => Ok(project_ref.get_installable(system)),
            EnvironmentRef::Named(named_ref) => {
                let gen = named_ref.get_current_gen(system).await?;
                Ok(named_ref.get_installable(flox, system, &gen))
            },
        }
    }
}
