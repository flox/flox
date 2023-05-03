use std::fmt::Display;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use log::debug;
use runix::command::{Eval, FlakeMetadata};
use runix::flake_ref::git::{GitAttributes, GitRef};
use runix::flake_ref::path::PathRef;
use runix::flake_ref::FlakeRef;
use runix::installable::FlakeAttribute;
use runix::{NixBackend, RunJson};
use thiserror::Error;
use url::Url;

use super::environment::CommonEnvironment;
use super::flox_installable::{FloxInstallable, ParseFloxInstallableError};
use super::floxmeta::{self, Floxmeta, GetFloxmetaError};
use super::project::{self, OpenProjectError};
use super::root::reference::ProjectDiscoverGitError;
use super::root::transaction::{GitAccess, ReadOnly};
use crate::flox::{Flox, FloxNixApi, ResolveFloxInstallableError};
use crate::providers::git::GitProvider;

static DEFAULT_NAME: &str = "default";
pub static DEFAULT_OWNER: &str = "local";

#[derive(Debug)]
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

#[derive(Debug)]
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

    fn meta_dir(flox: &Flox) -> PathBuf {
        flox.cache_dir.join("meta")
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

    /// Convert an environment reference to an installable
    fn get_installable(&self, flox: &Flox, system: &str, gen: &str) -> FlakeAttribute {
        let flakeref = FlakeRef::GitPath(GitRef {
            // we can unwrap here since we construct and know the path
            url: Url::from_file_path(Self::meta_dir(flox).join(&self.owner))
                .unwrap()
                .try_into()
                .unwrap(),
            attributes: GitAttributes {
                reference: format!("{system}.{name}", name = self.name).into(),
                dir: Path::new(gen).to_path_buf().into(),
                ..Default::default()
            },
        });

        FlakeAttribute {
            flakeref,
            // The git branch varies but the name always remains `default`,
            // which comes from the template
            // https://github.com/flox/flox-bash-private/tree/main/lib/templateFloxEnv/pkgs/default
            // and does not get renamed.
            //
            // enforce exact attr path (<flakeref>#.floxEnvs.<system>.default)
            attr_path: ["", "floxEnvs", system, "default"].try_into().unwrap(),
        }
    }

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

#[derive(Debug)]
pub enum EnvironmentRef<'flox> {
    Named(Named),
    Project(Project<'flox>),
}

impl Display for EnvironmentRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvironmentRef::Named(Named { owner, name }) => {
                if owner != DEFAULT_OWNER {
                    write!(f, "{owner}/")?
                }
                write!(f, "{owner}/{name}")
            },
            EnvironmentRef::Project(Project { workdir, name, .. }) => {
                write!(f, "{name} {workdir:?}")
            },
        }
    }
}

#[derive(Error, Debug)]
pub enum EnvironmentRefError<Git: GitProvider, Nix: FloxNixApi>
where
    Eval: RunJson<Nix>,
{
    #[error("Error finding project environment: {0}")]
    Project(FindProjectError<Nix>),
    #[error("Error finding named environment: {0}")]
    Named(FindNamedError<Git>),
    #[error("Name format is invalid")]
    Invalid,
}

#[allow(unused)]
impl EnvironmentRef<'_> {
    /// Returns a list of all matches for a user specified environment, including both named and
    /// project environment matches
    pub async fn find<'flox, Nix: FloxNixApi, Git: GitProvider>(
        flox: &'flox Flox,
        environment_name: Option<&str>,
    ) -> Result<(Vec<EnvironmentRef<'flox>>), EnvironmentRefError<Git, Nix>>
    where
        Eval: RunJson<Nix>,
        FlakeMetadata: RunJson<Nix>,
    {
        let (not_proj, not_named) = match environment_name {
            Some(name) => {
                debug!("Finding environment for {}", name);

                // Assume packages do not have '/' in their name
                // This is a weak assumption that is "mostly" true
                let not_proj = name.contains('/');

                let not_named =
                    // Skip named resolution if name starts with floxEnvs. or .floxEnvs.
                    name.starts_with("floxEnvs.") || name.starts_with(".floxEnvs.")
                    // Don't allow # in named environments as they look like flakerefs
                    || name.contains('#');

                // houston we have a problem
                if not_proj && not_named {
                    return Err(EnvironmentRefError::Invalid);
                }
                (not_proj, not_named)
            },
            None => {
                debug!("Finding environments");
                (false, false)
            },
        };

        let mut environment_refs = Vec::new();

        if !not_proj {
            match Project::find::<Nix, Git>(flox, environment_name).await {
                Err(e @ (FindProjectError::NotInGitRepo | FindProjectError::NotProject)) => {
                    debug!("{}", e);
                },
                Err(err) => return Err(EnvironmentRefError::Project(err)),
                Ok(ps) => {
                    for p in ps {
                        environment_refs.push(EnvironmentRef::Project(p));
                    }
                },
            };
        }

        if !not_named {
            match Named::find(flox, environment_name).await {
                Err(err) => return Err(EnvironmentRefError::Named(err)),
                Ok(None) => {},
                Ok(Some(n)) => {
                    environment_refs.push(EnvironmentRef::Named(n));
                },
            };
        }

        Ok(environment_refs)
    }

    pub async fn get_latest_flake_attribute<'flox, Git: GitProvider>(
        &self,
        flox: &'flox Flox,
    ) -> Result<FlakeAttribute, NamedGetCurrentGenError<Git>> {
        match self {
            EnvironmentRef::Project(project_ref) => Ok(project_ref.flake_attribute.clone()),
            EnvironmentRef::Named(named_ref) => {
                let gen = named_ref.get_current_gen(flox).await?;
                Ok(named_ref.get_installable(flox, &flox.system, &gen))
            },
        }
    }

    pub async fn to_env<'flox, Git: GitProvider + 'flox, Nix: FloxNixApi>(
        &'flox self,
        flox: &'flox Flox,
    ) -> Result<CommonEnvironment<Git>, CastError<Git, Nix>>
    where
        Eval: RunJson<Nix>,
    {
        let env = match self {
            EnvironmentRef::Named(Named { owner, name }) => {
                let floxmeta = Floxmeta::get_floxmeta(flox, owner).await?;
                let environment = floxmeta.environment(name).await?;
                CommonEnvironment::Named(environment)
            },
            EnvironmentRef::Project(Project {
                flox,
                flake_attribute,
                workdir,
                name,
            }) => {
                let path = match &flake_attribute.flakeref {
                    runix::flake_ref::FlakeRef::Path(PathRef { path, .. }) => path.clone(),
                    runix::flake_ref::FlakeRef::GitPath(GitRef { url, .. }) => {
                        url.to_file_path().unwrap()
                    },
                    _ => Err(CastError::InvalidFlakeRef)?,
                };

                let git = flox
                    .resource(path)
                    .guard::<Git>()
                    .await?
                    .open()
                    .map_err(|_| CastError::NotFound(flake_attribute.to_string()))?;

                let project = git
                    .guard()
                    .await?
                    .open()
                    .map_err(|_| CastError::NotFound(flake_attribute.to_string()))?;

                let environment = project.environment(name).await?;

                CommonEnvironment::Project(environment)
            },
        };
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
