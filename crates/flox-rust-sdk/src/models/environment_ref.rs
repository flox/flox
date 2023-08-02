use std::convert::Infallible;
use std::fmt::Display;
use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::str::FromStr;

use log::debug;
use runix::command::{Eval, FlakeMetadata};
use runix::installable::FlakeAttribute;
use runix::{NixBackend, RunJson};
use thiserror::Error;

use super::environment::{DotFloxDir, Environment, EnvironmentError2, Read, State};
use super::flox_installable::{FloxInstallable, ParseFloxInstallableError};
use super::floxmeta::{self, Floxmeta, GetFloxmetaError};
use super::project::{self, OpenProjectError};
use super::root::reference::ProjectDiscoverGitError;
use super::root::transaction::{GitAccess, ReadOnly};
use crate::flox::{Flox, FloxNixApi, ResolveFloxInstallableError};
use crate::providers::git::GitProvider;

pub static DEFAULT_NAME: &str = "default";
pub static DEFAULT_OWNER: &str = "local";

#[derive(Debug, Clone)]
pub struct Project<'flox> {
    pub flox: &'flox Flox,
    pub flake_attribute: FlakeAttribute,
    pub workdir: PathBuf, // todo https://github.com/flox/runix/issues/7
    pub name: String,
}

#[derive(Error, Debug)]
pub enum FindProjectError<Nix: FloxNixApi>
where
    Eval: RunJson<Nix>,
{
    #[error("Error reading current dir path: {0}")]
    CurrentDir(io::Error),
    #[error("Error trying to discover Git repo")]
    DiscoverError,
    #[error("Not in a Git repository")]
    NotInGitRepo,
    #[error("Error checking for project")]
    OpenError,
    #[error("Git repository found is not a flox project")]
    NotProject,
    #[error("Git repository found is bare")]
    BareRepo,
    #[error("Missing a name")]
    NoName,
    #[error("Failed to parse as flox installable: {0}")]
    Parse(#[from] ParseFloxInstallableError),
    #[error("Workdir is not valid unicode")]
    WorkdirEncoding,
    #[error("Error attempting to resolve to installables: {0}")]
    ResolveFailure(ResolveFloxInstallableError<Nix>),
}

impl<'flox> Project<'flox> {
    /// Returns a list of project matches for a user specified environment
    pub async fn find<Nix: FloxNixApi, Git: GitProvider>(
        flox: &'flox Flox,
        environment_name: Option<&str>,
    ) -> Result<Vec<Project<'flox>>, FindProjectError<Nix>>
    where
        Eval: RunJson<Nix>,
        FlakeMetadata: RunJson<Nix>,
    {
        // Find the `Project` to use, erroring all the way if it is not in the perfect state.
        // TODO: further changes and integrations to make more flexible possible?
        let git_repo = flox
            .resource(std::env::current_dir().map_err(FindProjectError::CurrentDir)?)
            .guard::<Git>()
            .await
            .map_err(|_| FindProjectError::DiscoverError)?
            .open()
            .map_err(|_| FindProjectError::NotInGitRepo)?;

        let project = git_repo
            .guard()
            .await
            .map_err(|_| FindProjectError::OpenError)?
            .open()
            .map_err(|_| FindProjectError::NotProject)?;

        // TODO: it is easy to use `.path()` instead, but we do not know any default branch.
        // In the future we may want to handle this?
        let workdir = project.workdir().ok_or(FindProjectError::BareRepo)?;

        let workdir_str = workdir.to_str().ok_or(FindProjectError::WorkdirEncoding)?;

        let flox_installables = match environment_name {
            Some(name) => vec![name.parse::<FloxInstallable>()?],
            None => vec![],
        };

        let matches = flox
            .resolve_matches::<Nix, Git>(
                &flox_installables,
                &[&format!("git+file://{workdir_str}")],
                &[("floxEnvs", true)],
                true,
                None,
            )
            .await
            .map_err(FindProjectError::ResolveFailure)?;

        matches
            .into_iter()
            .map(|m| {
                Ok(Project {
                    flox,
                    workdir: workdir.to_owned(),
                    name: m.key.last().ok_or(FindProjectError::NoName)?.to_owned(),
                    flake_attribute: m.flake_attribute(),
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct Named {
    pub owner: String,
    pub name: String,
}

#[derive(Error, Debug)]
pub enum NamedGetCurrentGenError<Git: GitProvider> {
    #[error("Error printing metadata {0}")]
    Show(Git::ShowError),
    #[error("Metadata file is not valid unicode")]
    MetadataEncoding,
    #[error("Error parsing current generation from metadata: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("`currentGen` attribute is missing")]
    NoCurrentGen,
    #[error("`currentGen` attribute is wrong type")]
    BadCurrentGen,
    #[error("Failed to open floxmeta directory for environment")]
    GetFloxmeta(GetFloxmetaError<Git>),
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
    #[error("Error finding default owner: {0}")]
    DefaultOwnerSymlinkTarget(#[from] FindDefaultOwnerError),
    #[error("Error checking directory")]
    CheckEnvironmentError(std::io::Error),
    #[error("Not found")]
    NotFound,
    #[error("Cached Git directory is missing")]
    OwnerPath(std::io::Error),
    #[error("Failed to open floxmeta directory for environment")]
    GetFloxmeta(GetFloxmetaError<Git>),
}

impl<'flox> Named {
    /// Check if user specified environment matches a named environment
    pub async fn find<Git: GitProvider>(
        flox: &'flox Flox,
        environment: Option<&str>,
    ) -> Result<Option<Named>, FindNamedError<Git>> {
        let (owner, name) = match environment {
            None => {
                let default_owner = Self::find_default_owner(flox).await?;
                (default_owner, DEFAULT_NAME.to_string())
            },
            Some(e) => match e.rsplit_once('/') {
                None => {
                    let default_owner = Self::find_default_owner(flox).await?;

                    (
                        default_owner,
                        if e.is_empty() {
                            DEFAULT_NAME.to_string()
                        } else {
                            e.to_string()
                        },
                    )
                },
                Some((owner, "")) => (owner.to_string(), DEFAULT_NAME.to_string()),
                Some((owner, name)) => (owner.to_string(), name.to_string()),
            },
        };

        // sanity check that floxmeta is valid
        match Floxmeta::<_, ReadOnly<Git>>::get_floxmeta(flox, &owner).await {
            Ok(_) => Ok(Some(Named { owner, name })),
            Err(GetFloxmetaError::NotFound(_)) => Ok(None),
            Err(e) => Err(FindNamedError::GetFloxmeta(e)),
        }
    }

    /// Return path to the environment data dir for an owner,
    /// e.g. ~/.local/share/flox/environments/owner
    pub fn associated_owner_dir(flox: &Flox, owner: &str) -> PathBuf {
        flox.data_dir.join("environments").join(owner)
    }

    /// Try to infer the name for the default owner.
    ///
    /// Installations of pacakges without an explicit owner are done for a pseudo owner
    /// called 'local'.
    /// Once a user is authenticated, and we know their username,
    /// the `local/*` environments are migrated
    /// and 'local' is linked to the the _actual_ `<user>` directory.
    ///
    /// This method tries to read the `local` link to infer the current owner name.
    ///
    /// Note: Username tracking is likely to change.
    async fn find_default_owner(flox: &Flox) -> Result<String, FindDefaultOwnerError> {
        let link_path = Self::associated_owner_dir(flox, DEFAULT_OWNER);
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

    #[allow(unused)] // contents might be useful later
    async fn get_current_gen<Git: GitProvider>(
        &self,
        flox: &'flox Flox,
    ) -> Result<String, NamedGetCurrentGenError<Git>> {
        let floxmeta = Floxmeta::<Git, ReadOnly<Git>>::get_floxmeta(flox, &self.owner)
            .await
            .map_err(NamedGetCurrentGenError::GetFloxmeta)?;
        let out_os_str = floxmeta
            .access
            .git()
            .show(&format!(
                "{system}.{name}:metadata.json",
                system = flox.system,
                name = self.name
            ))
            .await
            .map_err(|e| NamedGetCurrentGenError::Show(e))?;

        let out_str = out_os_str
            .to_str()
            .ok_or(NamedGetCurrentGenError::MetadataEncoding)?;

        let out: serde_json::Value = serde_json::from_str(out_str)?;

        Ok(out
            .get("currentGen")
            .ok_or(NamedGetCurrentGenError::NoCurrentGen)?
            .as_str()
            .ok_or(NamedGetCurrentGenError::BadCurrentGen)?
            .to_owned())
    }
}

#[derive(Debug, Clone)]
pub struct EnvironmentRef {
    owner: Option<String>,
    name: String,
}

impl Display for EnvironmentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref owner) = self.owner {
            write!(f, "{owner}/")?;
        }
        write!(f, "{}", self.name)
    }
}

impl FromStr for EnvironmentRef {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((owner, name)) = s.split_once('/') {
            Ok(Self {
                owner: Some(owner.to_string()),
                name: name.to_string(),
            })
        } else {
            Ok(Self {
                owner: None,
                name: s.to_string(),
            })
        }
    }
}

impl<S: State> From<Environment<S>> for EnvironmentRef {
    fn from(env: Environment<S>) -> Self {
        EnvironmentRef {
            name: env.name().to_string(),
            owner: env.owner().map(ToString::to_string),
        }
    }
}

#[derive(Error, Debug)]
pub enum EnvironmentRefError {
    #[error(transparent)]
    Environment(EnvironmentError2),

    #[error("Name format is invalid")]
    Invalid,
}

#[allow(unused)]
impl EnvironmentRef {
    /// Returns a list of all matches for a user specified environment
    pub fn find(
        flox: &Flox,
        environment_name: Option<&str>,
    ) -> Result<(Vec<EnvironmentRef>), EnvironmentRefError> {
        let dot_flox_dir = DotFloxDir::discover(std::env::current_dir().unwrap())
            .map_err(EnvironmentRefError::Environment)?;

        let env_ref = environment_name.map(
            |n| n.parse::<EnvironmentRef>().unwrap(), /* infallible */
        );

        let mut environment_refs = dot_flox_dir
            .environments()
            .map_err(EnvironmentRefError::Environment)?;
        if let Some(env_ref) = env_ref {
            environment_refs.retain(|env| {
                if env_ref.owner.is_some() {
                    env_ref.owner.as_deref() == env.owner() && env_ref.name == env.name()
                } else {
                    env_ref.name == env.owner().unwrap_or_else(|| env.name())
                }
            });
        }

        Ok(environment_refs.into_iter().map(|env| env.into()).collect())
    }

    pub async fn get_latest_flake_attribute<'flox, Git: GitProvider>(
        &self,
        flox: &'flox Flox,
    ) -> Result<FlakeAttribute, EnvironmentRefError> {
        let env = self.to_env()?;
        Ok(env.flake_attribute(&flox.system))
    }

    pub fn to_env(&self) -> Result<Environment<Read>, EnvironmentRefError> {
        let dot_flox_dir = DotFloxDir::discover(std::env::current_dir().unwrap())
            .map_err(EnvironmentRefError::Environment)?;
        let env = dot_flox_dir
            .environment(self.owner.clone(), &self.name)
            .map_err(EnvironmentRefError::Environment)?;
        Ok(env)
    }
}

#[derive(Debug, Error)]
pub enum CastError<Git: GitProvider, Nix: NixBackend>
where
    Eval: RunJson<Nix>,
{
    #[error(transparent)]
    GetFloxmeta(#[from] GetFloxmetaError<Git>),
    #[error(transparent)]
    GetFloxmetaEnvironment(#[from] floxmeta::environment::GetEnvironmentError<Git>),
    #[error(transparent)]
    DiscoveGit(#[from] ProjectDiscoverGitError<Git>),
    #[error("Environment not found: {0}")]
    NotFound(String),
    #[error("Only local git repositories ('git+file://') are supported at the moment")]
    InvalidFlakeRef,
    #[error(transparent)]
    OpenProject(#[from] OpenProjectError),
    #[error(transparent)]
    GetProjectEnvironment(#[from] project::GetEnvironmentError<Nix>),
}
