use std::io::{self, ErrorKind};
use std::path::PathBuf;

use log::debug;
use runix::command::Eval;
use runix::installable::Installable;
use runix::RunJson;
use thiserror::Error;

use super::flox_installable::ParseFloxInstallableError;
use crate::flox::{Flox, FloxNixApi, ResolveFloxInstallableError};
use crate::providers::git::GitProvider;

static DEFAULT_NAME: &str = "default";
static DEFAULT_OWNER: &str = "local";

#[derive(Debug)]
pub struct Project<'flox> {
    pub flox: &'flox Flox,
    pub installable: Installable,
    pub workdir: PathBuf,
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
        environment_name: &str,
    ) -> Result<Vec<Project<'flox>>, FindProjectError<Nix>>
    where
        Eval: RunJson<Nix>,
    {
        // Find the `Project` to use, erroring all the way if it is not in the perfect state.
        // TODO: further changes and integrations to make more flexible possible?
        let git_repo = flox
            .project(std::env::current_dir().map_err(FindProjectError::CurrentDir)?)
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

        let matches = flox
            .resolve_matches(
                &[environment_name.parse()?],
                &[&format!("git+file://{}", workdir_str)],
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
                    installable: m.installable(),
                })
            })
            .collect()
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
    #[error("Error checking cached Git repository")]
    GitDiscoverError(Git::DiscoverError),
}

impl<Git: GitProvider> Named<Git> {
    /// Check if user specified environment matches a named environment
    pub async fn find(flox: &Flox, environment: &str) -> Result<Option<Self>, FindNamedError<Git>> {
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

        match tokio::fs::metadata(Self::environment_dir(flox, &owner, &name)).await {
            // no matches for this environment exist
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(FindNamedError::CheckEnvironmentError(err)),
            Ok(_) => {},
        };

        let git = Git::discover(
            tokio::fs::canonicalize(Self::meta_dir(flox).join(&owner))
                .await
                .map_err(FindNamedError::OwnerPath)?,
        )
        .await
        // if we found an environment in data_dir but there's no git repo in meta_dir, something's
        // wrong
        .map_err(FindNamedError::GitDiscoverError)?;

        Ok(Some(Named { owner, name, git }))
    }

    fn meta_dir(flox: &Flox) -> PathBuf {
        flox.cache_dir.join("meta")
    }

    /// Return path to an owner in data dir, e.g. ~/.local/share/flox/environments/owner
    fn owner_dir(flox: &Flox, owner: &str) -> PathBuf {
        flox.data_dir.join("environments").join(owner)
    }

    /// Return path to environment in data dir,
    /// e.g. ~/.local/share/flox/environments/owner/system.name
    fn environment_dir(flox: &Flox, owner: &str, name: &str) -> PathBuf {
        Self::owner_dir(flox, owner).join(format!("{}.{}", flox.system, name))
    }

    async fn find_default_owner(flox: &Flox) -> Result<String, FindDefaultOwnerError> {
        let link_path = Self::owner_dir(flox, DEFAULT_OWNER);
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
pub enum EnvironmentRef<'flox, Git: GitProvider> {
    Named(Named<Git>),
    Project(Project<'flox>),
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
impl<Git: GitProvider> EnvironmentRef<'_, Git> {
    /// Returns a list of all matches for a user specified environment, including both named and
    /// project environment matches
    pub async fn find<'flox, Nix: FloxNixApi>(
        flox: &'flox Flox,
        environment_name: &str,
    ) -> Result<(Vec<EnvironmentRef<'flox, Git>>), EnvironmentRefError<Git, Nix>>
    where
        Eval: RunJson<Nix>,
    {
        debug!("Finding environment for {}", environment_name);

        let mut environment_refs = Vec::new();

        // Assume packages do not have '/' in their name
        // This is a weak assumption that is "mostly" true
        let not_proj = environment_name.contains('/');

        let not_named =
            // Skip named resolution if name starts with floxEnvs. or .floxEnvs.
            environment_name.starts_with("floxEnvs.") || environment_name.starts_with(".floxEnvs.")
            // Don't allow # in named environments as they look like flakerefs
            || environment_name.contains('#');

        // houston we have a problem
        if not_proj && not_named {
            return Err(EnvironmentRefError::Invalid);
        }

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

    pub async fn get_latest_installable<'flox>(
        &self,
        flox: &'flox Flox,
    ) -> Result<Installable, NamedGetCurrentGenError<Git>> {
        match self {
            EnvironmentRef::Project(project_ref) => Ok(project_ref.installable.clone()),
            EnvironmentRef::Named(named_ref) => {
                let gen = named_ref.get_current_gen(&flox.system).await?;
                Ok(named_ref.get_installable(flox, &flox.system, &gen))
            },
        }
    }
}
