use std::io::{self, ErrorKind};
use std::path::PathBuf;

use log::{debug, log};
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
    #[error("Environment specified exists, but it is not in a Git repo")]
    NotInGitRepo,
    #[error("Error checking for project")]
    OpenError,
    #[error("Found Git repo, but it is not a project")]
    NotProject,
    #[error("Environment is in a Git repo, but it is bare")]
    BareRepo,
    #[error("Environment is missing a name")]
    NoName,
    #[error("Failed to parse as flox installable")]
    Parse(#[from] ParseFloxInstallableError),
    #[error("Workdir is not valid unicode")]
    WorkdirEncoding,
    #[error("Error attempting to resolve to installables")]
    ResolveFailure(ResolveFloxInstallableError<Nix>),
}

impl<'flox> Project<'flox> {
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
    #[error("Error printing named environment metadata for branch {0}: {1}")]
    Show(String, Git::ShowError),
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
            .map_err(|e| {
                NamedGetCurrentGenError::Show(format!("{system}.{name}", name = self.name), e)
            })?;

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
    #[error(transparent)]
    Project(FindProjectError<Nix>),
    #[error(transparent)]
    Named(FindNamedError<Git>),
    #[error("Environment name format is invalid")]
    Invalid,
}

#[allow(unused)]
impl<Git: GitProvider> EnvironmentRef<'_, Git> {
    /// Try to find a project environment matching the inputted name,
    /// if the dir is missing or is not a Git repo, then try as a named environment
    pub async fn find<'flox, Nix: FloxNixApi>(
        flox: &'flox Flox,
        environment_name: &str,
    ) -> Result<Vec<EnvironmentRef<'flox, Git>>, EnvironmentRefError<Git, Nix>>
    where
        Eval: RunJson<Nix>,
    {
        debug!("Finding environment for {}", environment_name);

        let mut environment_refs = Vec::new();

        let mut not_proj = false;
        let mut not_named = false;

        // Lets hope nobody manages to put one of these in their project environment names
        if environment_name.contains('/') {
            not_proj = true;
        }

        // I think starting with `.` is totally possible, but we're going to hope nobody will do it,
        // so we can use it as a marker to force project resolution.
        // Yes this completely goes against @tomberek's Nix patch to make this skip resolving
        // since it resolves anyway, but whatever lol.
        if environment_name.starts_with("floxEnvs.") || environment_name.starts_with('.') {
            not_named = true;
        }

        // houston we have a problem
        if not_proj && not_named {
            return Err(EnvironmentRefError::Invalid);
        }

        if !not_proj {
            match Project::find::<Nix, Git>(flox, environment_name).await {
                Err(FindProjectError::NotInGitRepo | FindProjectError::NotProject) => {
                    debug!("Not in a project Git repo, forcing named resolution");
                    not_proj = true;
                },
                Err(err) => return Err(EnvironmentRefError::Project(err)),
                Ok(ps) => {
                    for p in ps {
                        environment_refs.push(EnvironmentRef::Project(p));
                    }

                    not_proj = false;
                },
            };
        }

        match Named::find(flox, environment_name).await {
            // This might be a bit picky, but a lot less should go wrong with named environments,
            // so we can assume that errors are likely to be user errors,
            // which are likely to be usage errors.
            // i.e. missing, which are probably fine to ignore 🤷
            Err(err) if not_proj => return Err(EnvironmentRefError::Named(err)),
            Err(err) => {
                log!(
                    // This is unlikely to be something a user cares about,
                    // unless they are getting "not found" errors.
                    if environment_refs.is_empty() {
                        log::Level::Warn
                    } else {
                        log::Level::Debug
                    },
                    "Error finding named environment, ignoring since we are in a project: {:?}",
                    err
                )
            },
            Ok(n) => {
                environment_refs.push(EnvironmentRef::Named(n));
            },
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
