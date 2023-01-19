use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use async_recursion::async_recursion;
use log::{debug, trace};
use runix::installable::Installable;
use thiserror::Error;

use crate::actions::project;
use crate::flox::Flox;
use crate::providers::git::GitProvider;

static DEFAULT_NAME: &str = "default";
static DEFAULT_OWNER: &str = "local";

static DEFAULT_PROJECT_ENV_DIR: &str = "pkgs";

#[derive(Debug)]
pub struct Project<'flox, Git: GitProvider> {
    pub flox: &'flox Flox,
    pub project: project::Project<'flox, project::Open<Git>>,
    pub name: PathBuf,
    pub subdir: OsString,
}

#[derive(Error, Debug)]
pub enum FindProjectError {
    #[error("Error checking whether environment is a path: {0}")]
    TryExists(#[from] io::Error),
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
    #[error("Environment is in a Git repo, but it is bare")]
    BareRepo,
    #[error("Does not end in a name")]
    NoName,
}

/// Roughly canonicalize a path
///
/// Works similarly to `tokio::fs::canonicalize`,
/// except it will tolerate a path being missing by instead checking the parent (and so on).
///
/// It will keep a record of all components it had to remove before finding the file in `removed`
#[async_recursion]
async fn rough_canonicalize(path: &Path, removed: &mut Vec<OsString>) -> std::io::Result<PathBuf> {
    match tokio::fs::canonicalize(path).await {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let mut parts = path.iter().collect::<Vec<_>>();
            match parts.len() {
                0 => Err(err),
                len => {
                    let last = parts.remove(len - 1);
                    removed.push(last.to_owned());

                    let shortened = parts.into_iter().collect::<PathBuf>();

                    trace!("Re-trying canonicalize on parent: {shortened:?} {removed:?}",);
                    rough_canonicalize(&shortened, removed).await
                },
            }
        },
        Err(err) => Err(err),
        Ok(p) => Ok(p.join(removed.iter().collect::<PathBuf>())),
    }
}

impl<'flox, Git: GitProvider> Project<'flox, Git> {
    #[async_recursion]
    pub async fn find_dir(environment_path: &Path) -> Result<PathBuf, FindProjectError> {
        // We get better results out of canonicalization if this is an absolute path
        let environment_path: Cow<Path> = if environment_path.is_relative() {
            std::env::current_dir()?.join(environment_path).into()
        } else {
            environment_path.into()
        };

        trace!("Finding project dir for: {environment_path:?}");

        let mut removed = Vec::new();

        match rough_canonicalize(&environment_path, &mut removed).await {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Err(FindProjectError::Missing)
            },
            Err(err) => Err(FindProjectError::TryExists(err)),
            Ok(real_path) => {
                if removed.is_empty() {
                    Ok(real_path)
                } else {
                    trace!("Components removed for canonicalization: {removed:?}");

                    let removed_path: PathBuf = removed.into_iter().collect();

                    if !removed_path.starts_with(DEFAULT_PROJECT_ENV_DIR)
                        && !real_path.ends_with(DEFAULT_PROJECT_ENV_DIR)
                    {
                        trace!("no {DEFAULT_PROJECT_ENV_DIR} inbetween real and removed, adding and retrying");
                        Ok(Self::find_dir(
                            &real_path.join(DEFAULT_PROJECT_ENV_DIR).join(removed_path),
                        )
                        .await?)
                    } else {
                        Err(FindProjectError::Missing)
                    }
                }
            },
        }
    }

    pub async fn find(
        flox: &'flox Flox,
        environment_path_raw: &Path,
    ) -> Result<Project<'flox, Git>, FindProjectError> {
        let environment_path = Self::find_dir(environment_path_raw).await?;
        trace!("Found environment path: {environment_path:?}");

        // Find the `Project` to use, erroring all the way if it is not in the perfect state.
        // TODO: further changes and integrations to make more flexible possible?
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

        // Project logic uses your current active work tree, so no bares.
        let workdir = project.workdir().ok_or(FindProjectError::BareRepo)?;

        // The whole path except for the working directory it may start with.
        // i.e. `/home/me/my-repo/xyz/a/b/c` becomes `xyz/a/b/c`
        let sub_path = match environment_path.strip_prefix(workdir) {
            Ok(s) => s,
            Err(_) => &environment_path,
        };

        trace!("Determined environment sub-path to be: {sub_path:?}",);

        // The first component of the sub-path made above,
        // and if none is supplied then the default.
        // This makes the assumption that environments will never be more than 1 level deep,
        // which may not always be true in the future.
        let subdir = sub_path
            .iter()
            .next()
            .unwrap_or_else(|| OsStr::new(DEFAULT_PROJECT_ENV_DIR))
            .to_owned();

        // The name is everything except for the subdir defined above,
        // so strip that from the sub-path to get the name.
        // If for some reason it does not exist (i.e. we added the default),
        // then just ignore the error.
        //
        // This can be any length, so as to support nesting,
        // since an env in `pkgs/stuff/a/flox.nix` results in `floxEnvs.[system].stuff.a`,
        // and we construct the above installable later.
        let name = match sub_path.strip_prefix(&subdir) {
            Ok(x) => x.to_owned(),
            Err(_) => sub_path.to_owned(),
        };

        // The above `subdir` and `name` logic can give us an empty name
        // which results in a faulty attrpath.
        // Make this a hard error.
        if name == Path::new("") {
            return Err(FindProjectError::NoName);
        }

        Ok(Project {
            flox,
            project,
            subdir,
            name,
        })
    }

    fn get_installable(&self, system: &str) -> Installable {
        Installable {
            flakeref: format!(
                "git+file://{project_dir}",
                project_dir = self.project.path().display(),
            ),
            attr_path: format!(
                ".floxEnvs.{system}.{name}",
                // Split the name apart and build it together separated with `.` to make a full attrpath
                name = self
                    .name
                    .iter()
                    .map(|x| x.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(".")
            ),
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
            .map_err(NamedGetCurrentGenError::Show)?;

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
        debug!("Finding environment for {}", environment_name);

        let environment_path_raw = PathBuf::from(environment_name);
        match (
            Project::find(flox, &environment_path_raw).await,
            environment_path_raw.components().next(),
        ) {
            (
                Err(FindProjectError::Missing | FindProjectError::NotInGitRepo),
                Some(std::path::Component::Normal(_)),
            ) => {
                debug!("Couldn't find project environment, searching for named environment");

                Ok(EnvironmentRef::Named(
                    Named::find(flox, environment_name)
                        .await
                        .map_err(EnvironmentRefError::Named)?,
                ))
            },
            (Ok(p), _) => Ok(EnvironmentRef::Project(p)),
            (Err(err), _) => Err(EnvironmentRefError::Project(err)),
        }
    }

    pub async fn get_latest_installable(
        &self,
        flox: &Flox,
    ) -> Result<Installable, NamedGetCurrentGenError<Git>> {
        match self {
            EnvironmentRef::Project(project_ref) => Ok(project_ref.get_installable(&flox.system)),
            EnvironmentRef::Named(named_ref) => {
                let gen = named_ref.get_current_gen(&flox.system).await?;
                Ok(named_ref.get_installable(flox, &flox.system, &gen))
            },
        }
    }
}
