//! The directory structure for a path environment looks like this:
//! .flox/
//!     ENVIRONMENT_POINTER_FILENAME
//!     ENVIRONMENT_DIR_NAME/
//!         MANIFEST_FILENAME
//!         LOCKFILE_FILENAME
//!     PATH_ENV_GCROOTS_DIR_NAME/
//!         $system.$name (out link)

use std::ffi::OsStr;
use std::fs::{self};
use std::io::Write;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use flox_types::catalog::{EnvCatalog, System};
use log::debug;
use runix::arguments::eval::EvaluationArgs;
use runix::arguments::EvalArgs;
use runix::command::Eval;
use runix::command_line::NixCommandLine;
use runix::flake_ref::path::PathRef;
use runix::installable::FlakeAttribute;
use runix::RunJson;

use super::{
    copy_dir_recursive,
    EditResult,
    Environment,
    EnvironmentError2,
    EnvironmentPointer,
    InstallationAttempt,
    PathPointer,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
    ENV_BUILDER_BIN,
    GCROOTS_DIR_NAME,
    LOCKFILE_FILENAME,
};
use crate::flox::Flox;
use crate::models::environment::{
    global_manifest_path,
    LockedManifest,
    CATALOG_JSON,
    ENV_DIR_NAME,
    MANIFEST_FILENAME,
};
use crate::models::environment_ref::EnvironmentName;
use crate::models::manifest::{insert_packages, remove_packages};
use crate::models::search::PKGDB_BIN;

/// Struct representing a local environment
///
/// This environment performs transactional edits by first copying the environment
/// to a temporary directory, making changes there, and attempting to build the
/// environment. If the build succeeds, the edit is considered a success and the
/// original environment contents are overwritten with the contents of the temporary
/// directory.
///
/// The transaction status is captured via the `state` field.
#[derive(Debug)]
pub struct PathEnvironment<S> {
    /// Absolute path to the environment, typically `<...>/.flox`
    pub path: PathBuf,

    /// The temporary directory that this environment will use during transactions
    pub temp_dir: PathBuf,

    /// The associated [PathPointer] of this environment.
    ///
    /// Used to identify the environment.
    pub pointer: PathPointer,

    /// The transaction state
    pub state: S,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct Original;
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct Temporary;

/// A marker trait used to identify types that represent transaction states
pub trait TransactionState: Send + Sync {}
impl TransactionState for Original {}
impl TransactionState for Temporary {}

impl<S> PartialEq for PathEnvironment<S> {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl<S: TransactionState> PathEnvironment<S> {
    pub fn new(
        dot_flox: impl AsRef<Path>,
        pointer: PathPointer,
        temp_dir: impl AsRef<Path>,
        state: S,
    ) -> Result<Self, EnvironmentError2> {
        let env_path = dot_flox.as_ref().join(ENV_DIR_NAME);
        if !env_path.exists() {
            Err(EnvironmentError2::EnvNotFound)?;
        }

        if !env_path.join(MANIFEST_FILENAME).exists() {
            Err(EnvironmentError2::DirectoryNotAnEnv)?
        }

        Ok(Self {
            // path must be absolute as it is used to set FLOX_ENV
            path: dot_flox
                .as_ref()
                .canonicalize()
                .map_err(EnvironmentError2::EnvCanonicalize)?,
            pointer,
            temp_dir: temp_dir.as_ref().to_path_buf(),
            state,
        })
    }

    /// Makes a temporary copy of the environment so edits can be applied without modifying the original environment
    pub fn make_temporary(&self) -> Result<PathEnvironment<Temporary>, EnvironmentError2> {
        let transaction_dir =
            tempfile::tempdir_in(&self.temp_dir).map_err(EnvironmentError2::MakeSandbox)?;

        copy_dir_recursive(&self.path, &transaction_dir, true)
            .map_err(EnvironmentError2::MakeTemporaryEnv)?;

        PathEnvironment::new(
            transaction_dir.into_path(),
            self.pointer.clone(),
            self.temp_dir.clone(),
            Temporary,
        )
    }

    /// Replace the contents of this environment's `.flox` with that of another environment's `.flox`
    ///
    /// This may copy build symlinks, so the assumption is that building self
    /// will result in the same out link as building the replacement.
    pub fn replace_with(
        &mut self,
        replacement: PathEnvironment<Temporary>,
    ) -> Result<(), EnvironmentError2> {
        let transaction_backup = self
            .path
            .with_file_name(format!("{}.tmp", self.name().as_ref()));
        if transaction_backup.exists() {
            debug!(
                "transaction backup exists: {}",
                transaction_backup.display()
            );
            return Err(EnvironmentError2::PriorTransaction(transaction_backup));
        }
        debug!(
            "backing up env: from={}, to={}",
            self.path.display(),
            transaction_backup.display()
        );
        fs::rename(&self.path, &transaction_backup)
            .map_err(EnvironmentError2::BackupTransaction)?;
        // try to restore the backup if the move fails
        debug!(
            "replacing original env: from={}, to={}",
            replacement.path.display(),
            self.path.display()
        );
        if let Err(err) = copy_dir_recursive(&replacement.path, &self.path, true) {
            debug!(
                "failed to replace env ({}), restoring backup: from={}, to={}",
                err,
                transaction_backup.display(),
                self.path.display(),
            );
            fs::remove_dir_all(&self.path).map_err(EnvironmentError2::AbortTransaction)?;
            fs::rename(transaction_backup, &self.path)
                .map_err(EnvironmentError2::AbortTransaction)?;
            return Err(EnvironmentError2::Move(err));
        }
        debug!("removing backup: path={}", transaction_backup.display());
        fs::remove_dir_all(transaction_backup).map_err(EnvironmentError2::RemoveBackup)?;
        Ok(())
    }

    /// Where to link a built environment to. The path may not exist if the environment has
    /// never been built.
    ///
    /// The existence of this path guarantees exactly two things:
    /// - The environment was built at some point in the past.
    /// - The environment can be activated.
    ///
    /// The existence of this path explicitly _does not_ guarantee that the current
    /// state of the environment is "buildable". The environment may have been modified
    /// since it was last built and therefore may no longer build. Thus, the presence of
    /// this path doesn't guarantee that the current environment can be built,
    /// just that it built at some point in the past.
    fn out_link(&self, system: &System) -> Result<PathBuf, EnvironmentError2> {
        let run_dir = self.path.join(GCROOTS_DIR_NAME);
        if !run_dir.exists() {
            std::fs::create_dir_all(&run_dir).map_err(EnvironmentError2::CreateGcRootDir)?;
        }
        Ok(run_dir.join([system.clone(), self.name().to_string()].join(".")))
    }
}

impl PathEnvironment<Temporary> {
    /// Updates the environment manifest with the provided contents
    pub fn update_manifest(&mut self, contents: &impl AsRef<str>) -> Result<(), EnvironmentError2> {
        debug!("writing new manifest to {}", self.manifest_path().display());
        let mut manifest_file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(self.manifest_path())
            .map_err(EnvironmentError2::OpenManifest)?;
        manifest_file
            .write_all(contents.as_ref().as_bytes())
            .map_err(EnvironmentError2::UpdateManifest)?;
        Ok(())
    }
}

#[async_trait]
impl<S> Environment for PathEnvironment<S>
where
    S: TransactionState,
{
    /// Build the environment with side effects:
    ///
    /// - Create a result link as gc-root.
    /// - Create a lockfile if one doesn't already exist, updating it with
    ///   any new packages.
    async fn build(&mut self, flox: &Flox) -> Result<(), EnvironmentError2> {
        debug!("building project environment at {}", self.path.display());
        let manifest_path = self.manifest_path();
        let lockfile_path = self.lockfile_path();
        let maybe_lockfile = if lockfile_path.exists() {
            debug!("found existing lockfile: {}", lockfile_path.display());
            Some(lockfile_path.as_ref())
        } else {
            debug!("no existing lockfile found");
            None
        };
        let lockfile = LockedManifest::lock_manifest(
            Path::new(&*PKGDB_BIN),
            &manifest_path,
            maybe_lockfile,
            &global_manifest_path(flox),
        )?;
        debug!("generated lockfile, writing to {}", lockfile_path.display());
        std::fs::write(&lockfile_path, lockfile.to_string())
            .map_err(EnvironmentError2::WriteLockfile)?;

        debug!(
            "building environment: system={}, lockfilePath={}",
            &flox.system,
            lockfile_path.display()
        );

        let store_path = lockfile.build(
            Path::new(&*ENV_BUILDER_BIN),
            Some(&self.out_link(&flox.system)?),
        )?;

        debug!(
            "built locked environment, store path={}",
            store_path.display()
        );

        // TODO: check the contents of the gc root or store path to see if it's empty
        // TODO: separate building and linking

        Ok(())
    }

    /// Install packages to the environment atomically
    ///
    /// Returns the new manifest content if the environment was modified. Also
    /// returns a map of the packages that were already installed. The installation
    /// will proceed if at least one of the requested packages were added to the
    /// manifest.
    async fn install(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError2> {
        let current_manifest_contents = self.manifest_content()?;
        let installation =
            insert_packages(&current_manifest_contents, &packages).map(|insertion| {
                InstallationAttempt {
                    new_manifest: insertion.new_toml.map(|toml| toml.to_string()),
                    already_installed: insertion.already_installed,
                }
            })?;
        if let Some(ref new_manifest) = installation.new_manifest {
            self.transact_with_manifest_contents(new_manifest, flox)
                .await?;
        }
        Ok(installation)
    }

    /// Uninstall packages from the environment atomically
    ///
    /// Returns true if the environment was modified and false otherwise.
    /// TODO: this should return a list of packages that were actually
    /// uninstalled rather than a bool.
    async fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<String, EnvironmentError2> {
        let current_manifest_contents = self.manifest_content()?;
        let toml = remove_packages(&current_manifest_contents, &packages)?;
        self.transact_with_manifest_contents(toml.to_string(), flox)
            .await?;
        Ok(toml.to_string())
    }

    /// Atomically edit this environment, ensuring that it still builds
    async fn edit(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<EditResult, EnvironmentError2> {
        let old_contents = self.manifest_content()?;
        // TODO we should probably skip this if the manifest hasn't changed
        self.transact_with_manifest_contents(&contents, flox)
            .await?;

        EditResult::new(&old_contents, &contents)
    }

    /// Get a catalog of installed packages from this environment
    ///
    /// Evaluated using nix from the environment definition.
    async fn catalog(
        &self,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<EnvCatalog, EnvironmentError2> {
        let mut flake_attribute = self.flake_attribute(system);
        flake_attribute.attr_path.push_attr("catalog").unwrap(); // valid attribute name, should not fail

        let eval = Eval {
            eval: EvaluationArgs {
                impure: true.into(),
                ..Default::default()
            },
            eval_args: EvalArgs {
                installable: Some(flake_attribute.into()),
                apply: None,
            },
            ..Eval::default()
        };

        let catalog_value: serde_json::Value = eval
            .run_json(nix, &Default::default())
            .await
            .map_err(EnvironmentError2::EvalCatalog)?;

        std::fs::write(self.catalog_path(), catalog_value.to_string())
            .map_err(EnvironmentError2::WriteCatalog)?;
        serde_json::from_value(catalog_value).map_err(EnvironmentError2::ParseCatalog)
    }

    fn manifest_content(&self) -> Result<String, EnvironmentError2> {
        fs::read_to_string(self.manifest_path()).map_err(EnvironmentError2::ReadManifest)
    }

    /// Returns the environment name
    fn name(&self) -> EnvironmentName {
        self.pointer.name.clone()
    }

    /// Delete the Environment
    fn delete(self) -> Result<(), EnvironmentError2> {
        let dot_flox = &self.path;
        if Some(OsStr::new(".flox")) == dot_flox.file_name() {
            std::fs::remove_dir_all(dot_flox).map_err(EnvironmentError2::DeleteEnvironment)?;
        } else {
            return Err(EnvironmentError2::DotFloxNotFound);
        }
        Ok(())
    }

    async fn activation_path(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        self.build(flox).await?;
        Ok(self.out_link(&flox.system)?)
    }

    fn parent_path(&self) -> Result<PathBuf, EnvironmentError2> {
        let mut path = self.path.clone();
        if path.pop() {
            Ok(path)
        } else {
            Err(EnvironmentError2::InvalidPath(path))
        }
    }
}

impl<S: TransactionState> PathEnvironment<S> {
    /// Turn the environment into a flake attribute,
    /// a precise url to interact with the environment via nix
    fn flake_attribute(&self, system: impl AsRef<str>) -> FlakeAttribute {
        let flakeref = PathRef {
            path: self.path.clone(),
            attributes: Default::default(),
        }
        .into();

        let attr_path = ["", "floxEnvs", system.as_ref(), "default"]
            .try_into()
            .unwrap(); // validated attributes

        FlakeAttribute {
            flakeref,
            attr_path,
            outputs: Default::default(),
        }
    }

    /// Path to the environment definition file
    pub fn manifest_path(&self) -> PathBuf {
        self.path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME)
    }

    /// Path to the lockfile. The path may not exist.
    pub fn lockfile_path(&self) -> PathBuf {
        self.path.join(ENV_DIR_NAME).join(LOCKFILE_FILENAME)
    }

    /// Path to the environment's catalog
    fn catalog_path(&self) -> PathBuf {
        self.path.join("pkgs").join("default").join(CATALOG_JSON)
    }

    /// Attempt to transactionally replace the manifest contents
    async fn transact_with_manifest_contents(
        &mut self,
        manifest_contents: impl AsRef<str>,
        flox: &Flox,
    ) -> Result<(), EnvironmentError2> {
        debug!("transaction: making temporary environment");
        let mut temp_env = self.make_temporary()?;
        debug!("transaction: updating manifest");
        temp_env.update_manifest(&manifest_contents)?;
        debug!("transaction: building environment");
        temp_env.build(flox).await?;
        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;
        Ok(())
    }
}

/// Constructors of PathEnvironments
impl PathEnvironment<Original> {
    /// Open an environment at a given path
    ///
    /// Ensure that the path exists and contains files that "look" like an environment
    pub fn open(
        pointer: PathPointer,
        dot_flox_path: impl AsRef<Path>,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError2> {
        let dot_flox = dot_flox_path.as_ref();
        log::debug!("attempting to open .flox directory: {}", dot_flox.display());
        if !dot_flox.exists() {
            Err(EnvironmentError2::DotFloxNotFound)?;
        }

        PathEnvironment::new(dot_flox, pointer, temp_dir, Original)
    }

    /// Create a new env in a `.flox` directory within a specific path or open it if it exists.
    ///
    /// The method creates or opens a `.flox` directory _contained_ within `path`!
    pub fn init(
        pointer: PathPointer,
        dot_flox_parent_path: impl AsRef<Path>,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError2> {
        match EnvironmentPointer::open(dot_flox_parent_path.as_ref()) {
            Err(EnvironmentError2::EnvNotFound) => {},
            Err(e) => Err(e)?,
            Ok(_) => Err(EnvironmentError2::EnvironmentExists)?,
        }
        let dot_flox_path = dot_flox_parent_path.as_ref().join(DOT_FLOX);
        let env_dir = dot_flox_path.join(ENV_DIR_NAME);
        debug!("creating env dir: {}", env_dir.display());
        std::fs::create_dir_all(&env_dir).map_err(EnvironmentError2::InitEnv)?;
        let pointer_content =
            serde_json::to_string_pretty(&pointer).map_err(EnvironmentError2::SerializeEnvJson)?;
        let template_path = env!("FLOX_ENV_TEMPLATE");
        debug!(
            "copying environment template from {} to {}",
            template_path,
            env_dir.display()
        );
        copy_dir_recursive(&template_path, &env_dir, false).map_err(EnvironmentError2::InitEnv)?;

        // Write the `env.json` file
        if let Err(e) = fs::write(
            dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME),
            pointer_content,
        ) {
            fs::remove_dir_all(env_dir).map_err(EnvironmentError2::InitEnv)?;
            Err(EnvironmentError2::WriteEnvJson(e))?;
        }

        Self::open(pointer, dot_flox_path, temp_dir)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    #[cfg(feature = "impure-unit-tests")]
    use crate::flox::tests::flox_instance;

    #[test]
    fn create_env() {
        let temp_dir = tempfile::tempdir().unwrap();
        let environment_temp_dir = tempfile::tempdir().unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let before = PathEnvironment::<Original>::open(
            pointer.clone(),
            environment_temp_dir.path(),
            temp_dir.path(),
        );

        assert!(
            matches!(before, Err(EnvironmentError2::EnvNotFound)),
            "{before:?}"
        );

        let actual = PathEnvironment::<Original>::init(
            pointer,
            environment_temp_dir.path(),
            temp_dir.path(),
        )
        .unwrap();

        let expected = PathEnvironment::new(
            environment_temp_dir.into_path().join(".flox"),
            PathPointer::new("test".parse().unwrap()),
            temp_dir.path(),
            Original,
        )
        .unwrap();

        assert_eq!(actual, expected);

        assert!(
            actual.path.join(ENV_DIR_NAME).join("flake.nix").exists(),
            "flake does not exist"
        );
        assert!(actual.manifest_path().exists(), "manifest exists");
        assert!(
            actual
                .path
                .join(ENV_DIR_NAME)
                .join("pkgs")
                .join("default")
                .join("default.nix")
                .exists(),
            "default.nix exists"
        );
        assert!(actual.path.is_absolute());
    }

    #[test]
    fn flake_attribute() {
        let temp_dir = tempfile::tempdir().unwrap();
        let environment_temp_dir = tempfile::tempdir().unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let env =
            PathEnvironment::<Original>::init(pointer, environment_temp_dir, temp_dir).unwrap();

        assert_eq!(
            env.flake_attribute("aarch64-darwin").to_string(),
            format!(
                "path:{}#.floxEnvs.aarch64-darwin.default",
                env.path.to_string_lossy()
            )
        )
    }

    #[tokio::test]
    #[cfg(feature = "impure-unit-tests")]
    async fn edit_env() {
        let (_flox, tempdir) = flox_instance();
        let pointer = PathPointer::new("test".parse().unwrap());

        let sandbox_path = tempdir.path().join("sandbox");
        std::fs::create_dir(&sandbox_path).unwrap();

        let mut env = PathEnvironment::init(pointer, &tempdir, &sandbox_path).unwrap();

        let mut temp_env = env.make_temporary().unwrap();

        assert_eq!(
            temp_env.path.parent().unwrap(),
            sandbox_path.canonicalize().unwrap()
        );

        let new_env_str = r#"
        { }
        "#;

        temp_env.update_manifest(&new_env_str).unwrap();

        assert_eq!(temp_env.manifest_content().unwrap(), new_env_str);

        env.replace_with(temp_env).unwrap();

        assert_eq!(env.manifest_content().unwrap(), new_env_str);
    }
}
