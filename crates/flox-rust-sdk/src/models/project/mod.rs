use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use log::info;
use runix::arguments::{EvalArgs, NixArgs};
use runix::command::{Eval, FlakeInit};
use runix::flake_ref::git::{GitAttributes, GitRef};
use runix::flake_ref::indirect::IndirectRef;
use runix::flake_ref::FlakeRef;
use runix::installable::{FlakeAttribute, Installable};
use runix::{NixBackend, Run, RunJson};
use tempfile::TempDir;
use thiserror::Error;
use walkdir::WalkDir;

use self::environment::Environment;
use super::root::transaction::{GitAccess, GitSandBox, ReadOnly};
use super::root::{Closed, Root};
use crate::flox::{Flox, FloxNixApi};
use crate::providers::git::GitProvider;
use crate::utils::errors::IoError;
use crate::utils::guard::Guard;
use crate::utils::{copy_file_without_permissions, find_and_replace, FindAndReplaceError};

pub mod environment;

static PACKAGE_NAME_PLACEHOLDER: &str = "__PACKAGE_NAME__";

#[derive(Debug)]
/// A representation of a project, i.e. a git repo with a flake.nix
///
/// We assume the flake.nix follows the capacitor output schema
pub struct Project<'flox, Git: GitProvider, Access: GitAccess<Git>> {
    flox: &'flox Flox,
    git: Access,
    /// subdir relative to the git workdir
    ///
    /// Represent setups where the project is not in the git root,
    /// or is a subflake.
    /// One such place is named env's generations:
    ///
    /// ```ignore
    /// /
    /// L .git/
    /// L 1/
    ///   L flake.nix
    ///   L pkgs/
    ///     L default/
    ///       L flox.nix
    /// ```
    subdir: PathBuf,
    _marker: PhantomData<Git>,
}

/// Upgrade paths from a git repo into an open Project
impl<'flox, Git: GitProvider> Root<'flox, Closed<Git>> {
    /// Guards opening a project
    ///
    /// - Resolves as initialized if a `flake.nix` is present
    /// - Resolves as uninitialized if not
    pub async fn guard(
        self,
    ) -> Result<Guard<Project<'flox, Git, ReadOnly<Git>>, Root<'flox, Closed<Git>>>, OpenProjectError>
    {
        let repo = &self.state.inner;

        let root = repo.workdir().ok_or(OpenProjectError::WorkdirNotFound)?;

        // todo: inset
        if root.join("flake.nix").exists() {
            Ok(Guard::Initialized(Project::new(
                self.flox,
                ReadOnly::new(self.state.inner),
                PathBuf::new(),
            )))
        } else {
            Ok(Guard::Uninitialized(self))
        }
    }
}

/// Resolutions for unsucessful upgrades
impl<'flox, Git: GitProvider> Guard<Project<'flox, Git, ReadOnly<Git>>, Root<'flox, Closed<Git>>> {
    /// Initialize a new project in the workdir of a git root or return
    /// an existing project if it exists.
    pub async fn init_project<Nix: FloxNixApi>(
        self,
        nix_extra_args: Vec<String>,
    ) -> Result<Project<'flox, Git, ReadOnly<Git>>, InitProjectError<Nix, Git>>
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
            template: Some(
                FlakeAttribute {
                    flakeref: IndirectRef::new("flox".to_string(), Default::default()).into(),
                    attr_path: ["", "templates", "_init"].try_into().unwrap(),
                    outputs: Default::default(),
                }
                .into(),
            ),
            ..Default::default()
        }
        .run(&nix, &NixArgs {
            cwd: Some(root.to_path_buf()),
            ..Default::default()
        })
        .await
        .map_err(InitProjectError::NixInitBase)?;

        repo.add(&[Path::new("flake.nix")])
            .await
            .map_err(InitProjectError::GitAdd)?;

        Ok(Project::new(
            uninit.flox,
            ReadOnly::new(repo),
            PathBuf::new(),
        ))
    }
}

/// Implementations for an opened project (read only)
impl<'flox, Git: GitProvider, Access: GitAccess<Git>> Project<'flox, Git, Access> {
    /// Construct a new Project object
    ///
    /// Private in this module, as intialization through git guard is prefered
    /// to provide project guarantees.
    fn new(flox: &Flox, git: Access, subdir: PathBuf) -> Project<Git, Access> {
        Project {
            flox,
            git,
            subdir,
            _marker: PhantomData,
        }
    }

    /// Get the git root for a flake
    ///
    /// The flake itself may be in a subdir as returned by flake_root()
    pub fn workdir(&self) -> Option<&Path> {
        self.git.git().workdir()
    }

    /// Get the root directory of the project flake
    ///
    /// If the project is a subflake, returns the subflake directory
    pub fn flake_root(&self) -> Option<PathBuf> {
        match self.flakeref() {
            FlakeRef::GitPath(GitRef { url, attributes }) => Some(
                url.to_file_path()
                    .unwrap()
                    .join(attributes.dir.unwrap_or_default()),
            ),
            _ => todo!("handling of non-local projects not implemented"),
        }
    }

    /// Get a the directory for links of rendered enviroments
    ///
    /// ## Panics
    ///
    /// Panics if flake is not a local repository
    /// todo: handling of other flake refs
    pub(crate) fn environment_out_link_dir(&self) -> PathBuf {
        self.flake_root().unwrap().join(".flox").join("envs")
    }

    /// flakeref for the project
    // todo: base project on FlakeRefs
    pub fn flakeref(&self) -> FlakeRef {
        FlakeRef::GitPath(GitRef::new(
            url::Url::from_directory_path(self.workdir().unwrap())
                .unwrap() // we know the path
                .try_into()
                .unwrap(), // we know its protocol is "file",
            GitAttributes {
                dir: Some(self.subdir.clone()),
                ..Default::default()
            },
        ))
    }

    /// Add a new flox style package from a template.
    /// Uses `nix flake init` to retrieve files
    /// and postprocesses the generic templates.
    //
    // todo: move to mutable state
    pub async fn init_flox_package<Nix: FloxNixApi>(
        &self,
        nix_extra_args: Vec<String>,
        template: Installable,
        name: &str,
    ) -> Result<(), InitFloxPackageError<Nix, Git>>
    where
        FlakeInit: Run<Nix>,
    {
        let repo = self.git.git();

        let nix = self.flox.nix(nix_extra_args);

        let root = repo
            .workdir()
            .ok_or(InitFloxPackageError::WorkdirNotFound)?;

        FlakeInit {
            template: Some(template.clone().into()),
            ..Default::default()
        }
        .run(&nix, &NixArgs {
            cwd: root.to_path_buf().into(),
            ..NixArgs::default()
        })
        .await
        .map_err(InitFloxPackageError::NixInit)?;

        // Comment this out since we're using mkShell instead of
        // root-level flox.nix
        // TODO: really find a better way to not hardcode this
        // if template.to_string() == "flake:flox#.\"templates\".\"project\"" {
        //     repo.add(&[&root.join("flox.nix")])
        //         .await
        //         .map_err(InitFloxPackageError::GitAdd)?;
        // }
        for dir_name in ["pkgs", "shells"] {
            let old_path = root.join(dir_name).join(PACKAGE_NAME_PLACEHOLDER);
            if old_path.exists() {
                let new_path = root.join(dir_name).join(name);

                repo.mv(&old_path, &new_path)
                    .await
                    .map_err(InitFloxPackageError::GitMv)?;
                info!(
                    "moved: {} -> {}",
                    old_path.to_string_lossy(),
                    new_path.to_string_lossy()
                );

                // our minimal "templating" - Replace any occurrences of
                // PACKAGE_NAME_PLACEHOLDER with name
                find_and_replace(&new_path, PACKAGE_NAME_PLACEHOLDER, name)
                    .await
                    .map_err(InitFloxPackageError::<Nix, Git>::ReplacePackageName)?;

                repo.add(&[&new_path])
                    .await
                    .map_err(InitFloxPackageError::GitAdd)?;
            }
        }

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

    /// Get a particular environment by name
    /// (attr path once nested packages are implemented)
    pub async fn environment<Nix: FloxNixApi>(
        &self,
        name: &str,
    ) -> Result<Environment<'flox, Git, ReadOnly<Git>>, GetEnvironmentError<Nix>>
    where
        Eval: RunJson<Nix>,
    {
        let nix = self.flox.nix::<Nix>(Default::default());

        let nix_apply_expr = format!(
            r#"systems: (systems."{}" or {{}}) ? "{name}""#,
            self.flox.system
        );

        let eval = Eval {
            eval_args: EvalArgs {
                apply: Some(nix_apply_expr.into()),
                installable: Some(
                    FlakeAttribute {
                        flakeref: self.flakeref(),
                        attr_path: ["floxEnvs".to_string()].try_into().unwrap(),
                        outputs: Default::default(),
                    }
                    .into(),
                ),
            },
            ..Eval::default()
        };

        let has_env = eval
            .run_json(&nix, &Default::default())
            .await
            .map_err(GetEnvironmentError::GetEnvExists)?;
        let has_env: bool =
            serde_json::from_value(has_env).map_err(GetEnvironmentError::DecodeEval)?;

        if !has_env {
            return Err(GetEnvironmentError::NotFound);
        }

        Ok(Environment {
            name: name.to_string(),
            system: self.flox.system.clone(),
            project: Project::new(self.flox, self.git.read_only(), self.subdir.clone()),
        })
    }

    /// List environments in this project
    pub async fn environments<Nix: FloxNixApi>(
        &'flox self,
    ) -> Result<Vec<Environment<'flox, Git, ReadOnly<Git>>>, GetEnvironmentsError<Nix>>
    where
        Eval: RunJson<Nix>,
    {
        let nix = self.flox.nix::<Nix>(Default::default());

        let nix_apply_expr = format!(
            r#"systems: builtins.attrNames (systems."{}" or {{}})"#,
            self.flox.system
        );

        let eval = Eval {
            eval_args: EvalArgs {
                apply: Some(nix_apply_expr.into()),
                installable: Some(
                    FlakeAttribute {
                        flakeref: self.flakeref(),
                        attr_path: ["floxEnvs".to_string()].try_into().unwrap(),
                        outputs: Default::default(),
                    }
                    .into(),
                ),
            },
            ..Eval::default()
        };

        let names = eval.run_json(&nix, &Default::default()).await.unwrap();
        let names = serde_json::from_value::<Vec<String>>(names).unwrap();

        let envs = names
            .into_iter()
            .map(|name| Environment {
                name,
                system: self.flox.system.clone(),
                project: Project::new(self.flox, self.git.read_only(), self.subdir.clone()),
            })
            .collect();

        Ok(envs)
    }
}

/// Implementations exclusively for [ReadOnly] instances
impl<'flox, Git: GitProvider> Project<'flox, Git, ReadOnly<Git>> {
    pub async fn enter_transaction(
        self,
    ) -> Result<(Project<'flox, Git, GitSandBox<Git>>, Index), TransactionEnterError> {
        let transaction_temp_dir =
            TempDir::new_in(&self.flox.temp_dir).map_err(TransactionEnterError::CreateTempdir)?;

        let current_root = self.workdir().expect("only supports projects on FS");

        for entry in WalkDir::new(current_root).into_iter().skip(1) {
            let entry = entry.map_err(TransactionEnterError::Walkdir)?;
            let new_path = transaction_temp_dir
                .path()
                .join(entry.path().strip_prefix(current_root).unwrap());
            if entry.file_type().is_dir() {
                tokio::fs::create_dir(new_path)
                    .await
                    .map_err(TransactionEnterError::CopyDir)?;
            } else {
                copy_file_without_permissions(entry.path(), &new_path)
                    .await
                    .map_err(TransactionEnterError::CopyFile)?;
            }
        }

        let git = Git::discover(transaction_temp_dir.path()).await.unwrap();

        let sandbox = self.git.to_sandbox_in(transaction_temp_dir, git);

        Ok((
            Project {
                flox: self.flox,
                git: sandbox,
                subdir: self.subdir,
                _marker: PhantomData,
            },
            Index::default(),
        ))
    }
}

type Index = BTreeMap<PathBuf, FileAction>;
pub enum FileAction {
    Add,
    Delete,
}

/// Implementations exclusively for [GitSandBox]ed instances
impl<'flox, Git: GitProvider> Project<'flox, Git, GitSandBox<Git>> {
    pub async fn commit_transaction(
        self,
        index: Index,
        _message: &str,
    ) -> Result<Project<'flox, Git, ReadOnly<Git>>, TransactionCommitError<Git>> {
        let original = self.git.read_only();

        for (file, action) in index {
            match action {
                FileAction::Add => {
                    if let Some(parent) = file.parent() {
                        tokio::fs::create_dir_all(original.git().workdir().unwrap().join(parent))
                            .await
                            .unwrap();
                    }
                    tokio::fs::rename(
                        self.git.git().workdir().unwrap().join(&file),
                        original.git().workdir().unwrap().join(&file),
                    )
                    .await
                    .unwrap();

                    original.git().add(&[&file]).await.expect("should add file")
                },
                FileAction::Delete => {
                    original
                        .git()
                        .rm(
                            &[&file],
                            original.git().workdir().unwrap().join(&file).is_dir(),
                            false,
                            false,
                        )
                        .await
                        .expect("should remove path");
                },
            }
        }

        Ok(Project {
            flox: self.flox,
            git: original,
            subdir: self.subdir,
            _marker: PhantomData,
        })
    }

    /// create a new root
    pub async fn create_default_env(&self, index: &mut Index) {
        let path = Path::new("flox.nix").to_path_buf();
        tokio::fs::write(
            self.workdir().expect("only works with workdir").join(&path),
            include_str!("./flox.nix.in"),
        )
        .await
        .unwrap();
        index.insert(path, FileAction::Add);
    }
}

#[derive(Error, Debug)]
pub enum TransactionEnterError {
    #[error("Failed to create tempdir for transaction")]
    CreateTempdir(std::io::Error),
    #[error("Failed to walk over file: {0}")]
    Walkdir(walkdir::Error),
    #[error("Failed to copy dir")]
    CopyDir(std::io::Error),
    #[error("Failed to copy file")]
    CopyFile(IoError),
}
#[derive(Error, Debug)]
pub enum TransactionCommitError<Git: GitProvider> {
    GitCommit(Git::CommitError),
    GitPush(Git::PushError),
}

/// Errors occurring while trying to upgrade to an [`Open<Git>`] [Root]
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

#[derive(Error, Debug)]
pub enum GetEnvironmentError<Nix: NixBackend>
where
    Eval: RunJson<Nix>,
{
    #[error("Could evaluate whether flake has environment: {0}")]
    GetEnvExists(<Eval as RunJson<Nix>>::JsonError),
    #[error("Could not decode eval response: {0}")]
    DecodeEval(serde_json::Error),
    #[error("Environment not found")]
    NotFound,
}

#[derive(Error, Debug)]
pub enum GetEnvironmentsError<Nix: NixBackend>
where
    Eval: RunJson<Nix>,
{
    #[error("Could not read environments: {0}")]
    ListEnvironments(<Eval as RunJson<Nix>>::JsonError),
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::flox::tests::flox_instance;
    use crate::providers::git::GitCommandProvider;

    #[tokio::test]
    async fn fail_without_git() {
        let (flox, tempdir_handle) = flox_instance();

        let project_dir = tempfile::tempdir_in(tempdir_handle.path()).unwrap();

        flox.resource(project_dir.path().to_path_buf())
            .guard::<GitCommandProvider>()
            .await
            .expect("Finding dir should succeed")
            .open()
            .expect_err("should find empty dir");
    }

    #[tokio::test]
    async fn fail_without_flake_nix() {
        let (flox, tempdir_handle) = flox_instance();

        let project_dir = tempfile::tempdir_in(tempdir_handle.path()).unwrap();
        let _project_git = GitCommandProvider::init(project_dir.path(), false)
            .await
            .expect("should create git repo");

        flox.resource(project_dir.path().to_path_buf())
            .guard::<GitCommandProvider>()
            .await
            .expect("Finding dir should succeed")
            .open()
            .expect("should find git repo")
            .guard()
            .await
            .expect("Openeing project dir should succeed")
            .open()
            .expect_err("Should error without flake.nix");
    }

    #[cfg(feature = "impure-unit-tests")]
    #[tokio::test]
    async fn create_project() {
        let temp_home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", temp_home.path());

        let (flox, tempdir_handle) = flox_instance();

        let project_dir = tempfile::tempdir_in(tempdir_handle.path()).unwrap();
        let _project_git = GitCommandProvider::init(project_dir.path(), false)
            .await
            .expect("should create git repo");

        let project = flox
            .resource(project_dir.path().to_path_buf())
            .guard::<GitCommandProvider>()
            .await
            .expect("Finding dir should succeed")
            .open()
            .expect("should find git repo")
            .guard()
            .await
            .expect("Openeing project dir should succeed")
            .init_project(Vec::new())
            .await
            .expect("Should init a new project");

        let envs = project
            .environments()
            .await
            .expect("should find empty floxEnvs");
        assert!(envs.is_empty());

        let (project, mut index) = project
            .enter_transaction()
            .await
            .expect("Should be able to make sandbox");

        project.create_default_env(&mut index).await;

        let project = project
            .commit_transaction(index, "unused")
            .await
            .expect("Should commit transaction");

        project
            .environment("default")
            .await
            .expect("should find new environment");
    }
}
