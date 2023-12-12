//! The directory structure for a path environment looks like this:
//!
//! ```ignore
//! .flox/
//!     ENVIRONMENT_POINTER_FILENAME
//!     ENVIRONMENT_DIR_NAME/
//!         MANIFEST_FILENAME
//!         LOCKFILE_FILENAME
//!     PATH_ENV_GCROOTS_DIR_NAME/
//!         $system.$name (out link)
//! ```
//!
//! `ENVIRONMENT_DIR_NAME` contains the environment definition
//! and is modified using [CoreEnvironment].

use std::ffi::OsStr;
use std::fs::{self};
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

use super::core_environment::CoreEnvironment;
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
    GCROOTS_DIR_NAME,
    LOCKFILE_FILENAME,
};
use crate::flox::Flox;
use crate::models::environment::{CATALOG_JSON, ENV_DIR_NAME, MANIFEST_FILENAME};
use crate::models::environment_ref::EnvironmentName;
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
pub struct PathEnvironment {
    /// Absolute path to the environment, typically `<...>/.flox`
    pub path: PathBuf,

    /// The temporary directory that this environment will use during transactions
    pub temp_dir: PathBuf,

    /// The associated [PathPointer] of this environment.
    ///
    /// Used to identify the environment.
    pub pointer: PathPointer,
}

impl PartialEq for PathEnvironment {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl PathEnvironment {
    pub fn new(
        dot_flox: impl AsRef<Path>,
        pointer: PathPointer,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError2> {
        let env_path = dot_flox.as_ref().join(ENV_DIR_NAME);
        if !env_path.exists() {
            Err(EnvironmentError2::EnvNotFound)?;
        }

        if !env_path.join(MANIFEST_FILENAME).exists() {
            Err(EnvironmentError2::ManifestNotFound)?
        }

        Ok(Self {
            // path must be absolute as it is used to set FLOX_ENV
            path: dot_flox
                .as_ref()
                .canonicalize()
                .map_err(EnvironmentError2::EnvCanonicalize)?,
            pointer,
            temp_dir: temp_dir.as_ref().to_path_buf(),
        })
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

    /// Get a view of the environment that can be used to perform operations
    /// on the environment without side effects.
    ///
    /// This method should only be used to create [CoreEnvironment]s for a [PathEnvironment].
    /// To modify the environment, use the [PathEnvironment] methods instead.
    pub(super) fn into_core_environment(self) -> CoreEnvironment {
        CoreEnvironment::new(self.path.join(ENV_DIR_NAME))
    }
}

#[async_trait]
impl Environment for PathEnvironment {
    /// Build the environment with side effects:
    ///
    /// - Create a result link as gc-root.
    /// - Create a lockfile if one doesn't already exist, updating it with
    ///   any new packages.
    async fn build(&mut self, flox: &Flox) -> Result<(), EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        env_view.build(flox)?;
        env_view.link(flox, self.out_link(&flox.system)?)?;

        Ok(())
    }

    /// Install packages to the environment atomically
    ///
    /// Returns the new manifest content if the environment was modified. Also
    /// returns a map of the packages that were already installed. The installation
    /// will proceed if at least one of the requested packages were added to the
    /// manifest.
    ///
    /// Todo: remove async
    async fn install(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.install(packages, flox)?;
        env_view.link(flox, self.out_link(&flox.system)?)?;

        Ok(result)
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
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.uninstall(packages, flox)?;
        env_view.link(flox, self.out_link(&flox.system)?)?;

        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    async fn edit(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<EditResult, EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.edit(flox, contents)?;
        env_view.link(flox, self.out_link(&flox.system)?)?;

        Ok(result)
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

    /// Read the environment definition file as a string
    fn manifest_content(&self, flox: &Flox) -> Result<String, EnvironmentError2> {
        fs::read_to_string(self.manifest_path(flox)?).map_err(EnvironmentError2::ReadManifest)
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

    /// Path to the environment definition file
    fn manifest_path(&self, _flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        Ok(self.path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME))
    }

    /// Path to the lockfile. The path may not exist.
    fn lockfile_path(&self, _flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        Ok(self.path.join(ENV_DIR_NAME).join(LOCKFILE_FILENAME))
    }
}

impl PathEnvironment {
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

    /// Path to the environment's catalog
    fn catalog_path(&self) -> PathBuf {
        self.path.join("pkgs").join("default").join(CATALOG_JSON)
    }
}

/// Constructors of PathEnvironments
impl PathEnvironment {
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

        PathEnvironment::new(dot_flox, pointer, temp_dir)
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
            Ok(_) => Err(EnvironmentError2::EnvironmentExists(
                dot_flox_parent_path.as_ref().to_path_buf(),
            ))?,
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
    use crate::flox::tests::flox_instance;

    #[test]
    fn create_env() {
        let (flox, temp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&temp_dir).unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let before = PathEnvironment::open(
            pointer.clone(),
            environment_temp_dir.path(),
            temp_dir.path(),
        );

        assert!(
            matches!(before, Err(EnvironmentError2::EnvNotFound)),
            "{before:?}"
        );

        let actual =
            PathEnvironment::init(pointer, environment_temp_dir.path(), temp_dir.path()).unwrap();

        let expected = PathEnvironment::new(
            environment_temp_dir.into_path().join(".flox"),
            PathPointer::new("test".parse().unwrap()),
            temp_dir.path(),
        )
        .unwrap();

        assert_eq!(actual, expected);

        assert!(
            actual.manifest_path(&flox).unwrap().exists(),
            "manifest exists"
        );
        assert!(actual.path.is_absolute());
    }

    #[test]
    fn flake_attribute() {
        let temp_dir = tempfile::tempdir().unwrap();
        let environment_temp_dir = tempfile::tempdir().unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let env = PathEnvironment::init(pointer, environment_temp_dir, temp_dir).unwrap();

        assert_eq!(
            env.flake_attribute("aarch64-darwin").to_string(),
            format!(
                "path:{}#.floxEnvs.aarch64-darwin.default",
                env.path.to_string_lossy()
            )
        )
    }
}
