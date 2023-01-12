use std::io::{self, ErrorKind};
use std::path::{PathBuf, StripPrefixError};

use log::{debug, trace};
use runix::arguments::eval::EvaluationArgs;
use runix::arguments::source::SourceArgs;
use runix::arguments::NixArgs;
use runix::command::Eval;
use runix::installable::Installable;
use runix::RunJson;
use thiserror::Error;

use crate::flox::{Flox, FloxNixApi};
use crate::providers::git::GitProvider;

static DEFAULT_NAME: &str = "default";
static DEFAULT_OWNER: &str = "local";

#[derive(Debug)]
pub struct ProjectEnvironmentRef {
    pub flake_dir: String,
    pub name: String,
}

#[derive(Error, Debug)]
pub enum ProjectEnvironmentRefError<DiscoverError> {
    #[error("environment specified exists, but it is not in a git repo: {0}")]
    NotInGitRepo(DiscoverError),
    #[error("environment is in a git repo, but it is bare")]
    BareRepo,
    // TODO be more informative about (or handle) symlinks or fs boundaries or something?
    #[error("environment is a part of git repo, but git repo is not a prefix of environment: {0}")]
    StripPrefix(StripPrefixError),
    #[error("failed to check if git repo contains a flake.nix: {0}")]
    TryExistsFlake(io::Error),
    #[error("environment is in git repo, but git repo does not contain a flake.nix")]
    NotFlake,
    #[error("environment is in flake repo, but not inside of pkgs/")]
    NotPkgs,
    #[error("Project path is not valid unicode")]
    ProjectPathEncoding,
}

impl ProjectEnvironmentRef {
    pub async fn new<Git: GitProvider>(
        environment_path: &PathBuf,
    ) -> Result<Self, ProjectEnvironmentRefError<Git::DiscoverError>> {
        trace!("Finding git repository for given path: {environment_path:?}");

        let git = Git::discover(environment_path)
            .await
            .map_err(ProjectEnvironmentRefError::NotInGitRepo)?;

        let repo_workdir = git.workdir().ok_or(ProjectEnvironmentRefError::BareRepo)?;

        trace!("Repo workdir for {environment_path:?} is {repo_workdir:?}");

        let is_flake = repo_workdir
            .join("flake.nix")
            .try_exists()
            .map_err(ProjectEnvironmentRefError::TryExistsFlake)?;

        if !is_flake {
            return Err(ProjectEnvironmentRefError::NotFlake);
        }

        let subdir = environment_path
            .strip_prefix(repo_workdir)
            .map_err(ProjectEnvironmentRefError::StripPrefix)?;

        let name = subdir
            .strip_prefix("pkgs")
            .map_err(|_| ProjectEnvironmentRefError::NotPkgs)?;

        Ok(ProjectEnvironmentRef {
            flake_dir: repo_workdir
                .to_str()
                .ok_or(ProjectEnvironmentRefError::ProjectPathEncoding)?
                .to_owned(),
            name: name
                .to_str()
                .ok_or(ProjectEnvironmentRefError::ProjectPathEncoding)?
                .to_owned(),
        })
    }

    fn get_installable(&self, system: &str) -> Installable {
        Installable {
            flakeref: format!("git+file://{flake_dir}", flake_dir = self.flake_dir),
            attr_path: format!("floxEnvs.{system}.{name}", name = self.name),
        }
    }
}

#[derive(Debug)]
pub struct NamedEnvironmentRef {
    pub owner: String,
    pub name: String,
}

#[derive(Error, Debug)]
pub enum NamedEnvironmentGetCurrentGenError<Nix: FloxNixApi>
where
    Eval: RunJson<Nix>,
{
    #[error("Error evaluating to read metadata: {0}")]
    Eval(<Eval as RunJson<Nix>>::JsonError),
    #[error("Error parsing current gen from metadata: {0}")]
    Parse(#[from] serde_json::Error),
}

#[derive(Error, Debug)]
pub enum NamedEnvironmentRefError {
    #[error("Symlink for default owner is invalid")]
    DefaultOwnerSymlinkTarget,
    #[error("Error checking symlink for default owner")]
    ReadLink(io::Error),
    #[error("Default owner symlink is not valid unicode")]
    DefaultOwnerSymlinkEncoding,
}

impl NamedEnvironmentRef {
    pub async fn new(flox: &Flox, environment: &str) -> Result<Self, NamedEnvironmentRefError> {
        // TODO: fallback prompt stuff
        let (owner, name) = match environment.rsplit_once('/') {
            None => {
                let default_owner = NamedEnvironmentRef::default_owner(flox).await?;

                return Ok(NamedEnvironmentRef {
                    owner: default_owner,
                    name: if environment.is_empty() {
                        DEFAULT_NAME.to_string()
                    } else {
                        environment.to_string()
                    },
                });
            },
            Some((owner, "")) => (owner.into(), DEFAULT_NAME.to_string()),
            Some((owner, name)) => (owner.into(), name.to_string()),
        };

        Ok(NamedEnvironmentRef { owner, name })
    }

    fn meta_dir(flox: &Flox) -> PathBuf {
        flox.cache_dir.join("meta")
    }

    async fn default_owner(flox: &Flox) -> Result<String, NamedEnvironmentRefError> {
        let link_path = NamedEnvironmentRef::meta_dir(flox).join(DEFAULT_OWNER);
        debug!(
            "Checking `local` symlink (`{}`) for true name of default user",
            link_path.display()
        );

        match tokio::fs::read_link(link_path).await {
            Ok(p) => Ok(p
                .file_name()
                .ok_or(NamedEnvironmentRefError::DefaultOwnerSymlinkTarget)?
                .to_str()
                .ok_or(NamedEnvironmentRefError::DefaultOwnerSymlinkEncoding)?
                .to_owned()),
            Err(err) => match err.kind() {
                // `InvalidInput` occurs if the path is not a symlink
                ErrorKind::NotFound | ErrorKind::InvalidInput => Ok(DEFAULT_OWNER.to_owned()),
                _ => Err(NamedEnvironmentRefError::ReadLink(err)),
            },
        }
    }

    fn get_installable(&self, flox: &Flox, system: &str, gen: &str) -> Installable {
        Installable {
            flakeref: format!(
                "git+file://{meta_dir}/{owner}?ref={system}.{name}&dir={gen}",
                name = self.name,
                owner = self.owner,
                meta_dir = NamedEnvironmentRef::meta_dir(flox).display(),
            ),
            attr_path: format!("floxEnvs.{system}.{name}", name = self.name),
        }
    }

    async fn get_current_gen<Nix: FloxNixApi>(
        &self,
        flox: &Flox,
        system: &str,
    ) -> Result<String, NamedEnvironmentGetCurrentGenError<Nix>>
    where
        Eval: RunJson<Nix>,
    {
        let expr = format!(
            r#"(builtins.fromJSON
                (builtins.readFile
                    "${{builtins.fetchGit {{ url = "file://{meta_dir}/{owner}?ref={system}.{name}"; ref = "{system}.{name}"; }}}}/metadata.json"
                )
            ).currentGen"#,
            name = self.name,
            owner = self.owner,
            meta_dir = NamedEnvironmentRef::meta_dir(flox).display(),
        );
        let command = Eval {
            source: SourceArgs {
                expr: Some(expr.into()),
            },
            eval: EvaluationArgs {
                impure: true.into(),
            },
            ..Default::default()
        };

        let json_out = command
            .run_json(&flox.nix::<Nix>(vec![]), &NixArgs::default())
            .await
            .map_err(NamedEnvironmentGetCurrentGenError::Eval)?;

        Ok(serde_json::from_value::<String>(json_out)?)
    }
}

#[derive(Debug)]
pub enum EnvironmentRef {
    Named(NamedEnvironmentRef),
    Project(ProjectEnvironmentRef),
}

#[derive(Error, Debug)]
pub enum EnvironmentRefError<DiscoverError> {
    #[error(transparent)]
    Project(ProjectEnvironmentRefError<DiscoverError>),
    #[error(transparent)]
    Named(NamedEnvironmentRefError),
    #[error("error checking whether environment is a path: {0}")]
    TryExists(io::Error),
}

#[allow(unused)]
impl EnvironmentRef {
    pub async fn new<Git: GitProvider>(
        flox: &Flox,
        environment_name: &str,
    ) -> Result<Self, EnvironmentRefError<Git::DiscoverError>> {
        let path = match PathBuf::from(environment_name).canonicalize() {
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => None,
                _ => return Err(EnvironmentRefError::TryExists(err)),
            },
            Ok(x) => Some(x),
        };

        if let Some(path) = path {
            Ok(EnvironmentRef::Project(
                ProjectEnvironmentRef::new::<Git>(&path)
                    .await
                    .map_err(EnvironmentRefError::Project)?,
            ))
        } else {
            Ok(EnvironmentRef::Named(
                NamedEnvironmentRef::new(flox, environment_name)
                    .await
                    .map_err(EnvironmentRefError::Named)?,
            ))
        }
    }

    pub async fn get_latest_installable<Nix: FloxNixApi>(
        &self,
        flox: &Flox,
    ) -> Result<Installable, NamedEnvironmentGetCurrentGenError<Nix>>
    where
        Eval: RunJson<Nix>,
    {
        debug!("Resolving env to installable: {:?}", self);

        let system = &flox.system;

        match self {
            EnvironmentRef::Project(project_ref) => Ok(project_ref.get_installable(system)),
            EnvironmentRef::Named(named_ref) => {
                let gen = named_ref.get_current_gen(flox, system).await?;
                Ok(named_ref.get_installable(flox, system, &gen))
            },
        }
    }
}
