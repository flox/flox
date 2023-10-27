use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use flox_types::catalog::{EnvCatalog, System};
use log::debug;
use runix::arguments::eval::EvaluationArgs;
use runix::arguments::{BuildArgs, EvalArgs};
use runix::command::{Build, Eval};
use runix::command_line::NixCommandLine;
use runix::flake_ref::path::PathRef;
use runix::installable::FlakeAttribute;
use runix::{Run, RunJson};

use super::{
    copy_dir_recursive,
    Environment,
    EnvironmentError2,
    EnvironmentPointer,
    InstallationAttempt,
    PathPointer,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
    MANIFEST_FILENAME,
};
use crate::models::environment::CATALOG_JSON;
use crate::models::environment_ref::{EnvironmentName, EnvironmentOwner, EnvironmentRef};
use crate::models::manifest::insert_packages;
use crate::prelude::flox_package::FloxPackage;
use crate::utils::copy_file_without_permissions;

const ENVIRONMENT_DIR_NAME: &'_ str = "env";

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
    /// Absolute path to the environment, typically within `<...>/.flox/name`
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
    /// Makes a temporary copy of the environment so edits can be applied without modifying the original environment
    pub fn make_temporary(&self) -> Result<PathEnvironment<Temporary>, EnvironmentError2> {
        let transaction_dir =
            tempfile::tempdir_in(&self.temp_dir).map_err(EnvironmentError2::MakeSandbox)?;

        copy_dir_recursive(&self.path, &transaction_dir, true)
            .map_err(EnvironmentError2::MakeTemporaryEnv)?;

        Ok(PathEnvironment {
            path: transaction_dir.into_path(),
            temp_dir: self.temp_dir.clone(),
            pointer: self.pointer.clone(),
            state: Temporary,
        })
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
            return Err(EnvironmentError2::PriorTransaction(transaction_backup));
        }
        fs::rename(&self.path, &transaction_backup)
            .map_err(EnvironmentError2::BackupTransaction)?;
        // try to restore the backup if the move fails
        if let Err(err) = fs::rename(replacement.path, &self.path) {
            fs::rename(transaction_backup, &self.path)
                .map_err(EnvironmentError2::AbortTransaction)?;
            return Err(EnvironmentError2::Move(err));
        }
        fs::remove_dir_all(transaction_backup).map_err(EnvironmentError2::RemoveBackup)?;
        Ok(())
    }

    /// Where to link a built environment to. The parent directory may not exist.
    ///
    /// When used as a lookup signals whether the environment has *at some point* been built before
    /// and is "activatable". Note that the environment may have been modified since it was last built.
    ///
    /// Mind that an existing out link does not necessarily imply that the environment
    /// can in fact be built.
    fn out_link(&self, system: impl AsRef<str> + Send) -> PathBuf {
        self.path
            .join("envs")
            .join(format!("{0}.{1}", system.as_ref(), self.name()))
    }
}

impl PathEnvironment<Temporary> {
    /// Updates the environment manifest with the provided contents
    pub fn update_manifest(&mut self, contents: &impl AsRef<str>) -> Result<(), EnvironmentError2> {
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
    /// - Copy catalog.json from the result into the environment. Whenever the
    /// environment is built, the lock is potentially updated. The lockfile is
    /// an input to the build and allows skipping relocking, so we copy it back
    /// into the environment to avoid relocking.
    async fn build(
        &mut self,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<(), EnvironmentError2> {
        debug!("building with nix ....");

        let out_link = self.out_link(&system);

        let build = Build {
            installables: [self.flake_attribute(&system).into()].into(),
            eval: runix::arguments::eval::EvaluationArgs {
                impure: true.into(),
                ..Default::default()
            },
            build: BuildArgs {
                out_link: Some(out_link.clone().into()),
                ..Default::default()
            },
            ..Default::default()
        };

        build
            .run(nix, &Default::default())
            .await
            .map_err(EnvironmentError2::Build)?;

        // environments potentially update their catalog in the process of a build because unlocked
        // packages (e.g. nixpkgs-flox.hello) must be pinned to a specific version which is added to
        // the catalog
        let result_catalog_json = out_link.join(CATALOG_JSON);
        copy_file_without_permissions(result_catalog_json, self.catalog_path())
            .map_err(EnvironmentError2::CopyFile)?;

        Ok(())
    }

    /// Install packages to the environment atomically
    ///
    /// Returns true if the environment was modified and false otherwise.
    /// TODO: this should return a list of packages that were actually
    /// installed rather than a bool.
    async fn install(
        &mut self,
        packages: Vec<String>,
        _nix: &NixCommandLine,
        _system: System,
    ) -> Result<InstallationAttempt, EnvironmentError2> {
        let current_manifest_contents = self.manifest_content()?;
        let installation = insert_packages(&current_manifest_contents, packages.iter().cloned())
            .map(|insertion| InstallationAttempt {
                new_manifest: insertion.new_toml.map(|toml| toml.to_string()),
                already_installed: insertion.already_installed,
            })?;
        if installation.new_manifest.is_some() {
            // TODO: enable transactions once build is re-implemented
            // self.transact_with_manifest_contents(toml.to_string(), nix, system).await?;
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
        _packages: Vec<FloxPackage>,
        _nix: &NixCommandLine,
        _system: System,
    ) -> Result<Option<String>, EnvironmentError2> {
        // let current_manifest_contents = self.manifest_content()?;

        // let new_manifest_contents =
        //     flox_nix_content_with_packages_removed(&current_manifest_contents, packages)?;
        // match new_manifest_contents {
        //     ManifestContent::Unchanged => return Ok(false),
        //     ManifestContent::Changed(contents) => {
        //         self.transact_with_manifest_contents(contents, nix, system)
        //             .await?;
        //         Ok(true)
        //     },
        // }
        todo!()
    }

    /// Atomically edit this environment, ensuring that it still builds
    async fn edit(
        &mut self,
        nix: &NixCommandLine,
        system: System,
        contents: String,
    ) -> Result<(), EnvironmentError2> {
        self.transact_with_manifest_contents(contents, nix, system)
            .await?;
        Ok(())
    }

    /// Return the [EnvironmentRef] for the environment for identification
    fn environment_ref(&self) -> EnvironmentRef {
        EnvironmentRef::new_from_parts(None, self.pointer.name.clone())
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

    /// Returns the environment owner
    fn owner(&self) -> Option<EnvironmentOwner> {
        None
    }

    /// Returns the environment name
    fn name(&self) -> EnvironmentName {
        self.pointer.name.clone()
    }

    /// Delete the Environment
    fn delete(self) -> Result<(), EnvironmentError2> {
        // `self.path` refers to `.flox/<env>`, so we check that the parent exists and is called
        // `.flox` before deleting the entire parent directory
        let Some(env_parent) = self.path.parent() else {
            return Err(EnvironmentError2::DotFloxNotFound);
        };
        if Some(OsStr::new(".flox")) == env_parent.file_name() {
            std::fs::remove_dir_all(env_parent).map_err(EnvironmentError2::DeleteEnvironment)?;
        } else {
            return Err(EnvironmentError2::DotFloxNotFound);
        }
        Ok(())
    }

    fn flake_attribute(&self, system: System) -> FlakeAttribute {
        self.flake_attribute(system)
    }
}

impl<S: TransactionState> PathEnvironment<S> {
    /// Turn the environment into a flake attribute,
    /// a precise url to interact with the environment via nix
    ///
    /// ```
    /// # use flox_rust_sdk::models::environment::PathPointer;
    /// # use flox_rust_sdk::models::environment::path_environment::{Original,PathEnvironment};
    /// # use std::path::PathBuf;
    /// # let tempdir = tempfile::tempdir().unwrap();
    /// # let environment_temp_dir = tempfile::tempdir().unwrap();
    /// # let path = tempdir.path().canonicalize().unwrap().to_string_lossy().into_owned();
    /// # let system = "aarch64-darwin";
    ///
    /// let env = PathEnvironment::<Original>::init(
    ///     PathPointer::new("test_env".parse().unwrap()),
    ///     &path,
    ///     environment_temp_dir.into_path(),
    /// )
    /// .unwrap();
    ///
    /// let flake_attribute = format!("path:{path}/.flox/env#.floxEnvs.{system}.default")
    ///     .parse()
    ///     .unwrap();
    /// assert_eq!(env.flake_attribute(system), flake_attribute)
    /// ```
    pub fn flake_attribute(&self, system: impl AsRef<str>) -> FlakeAttribute {
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
        self.path.join(MANIFEST_FILENAME)
    }

    /// Path to the environment's catalog
    fn catalog_path(&self) -> PathBuf {
        self.path.join("pkgs").join("default").join(CATALOG_JSON)
    }

    /// Attempt to transactionally replace the manifest contents
    async fn transact_with_manifest_contents(
        &mut self,
        manifest_contents: impl AsRef<str>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<(), EnvironmentError2> {
        let mut temp_env = self.make_temporary()?;
        temp_env.update_manifest(&manifest_contents)?;
        temp_env.build(nix, system).await?;
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
        let env_dir = dot_flox_path.as_ref().join(ENVIRONMENT_DIR_NAME);
        tracing::debug!(?env_dir, "attempting to open environment directory");
        if !env_dir.exists() {
            eprintln!("dir doesn't exist");
            Err(EnvironmentError2::EnvNotFound)?;
        }

        let env_path = env_dir
            .canonicalize()
            .map_err(EnvironmentError2::EnvCanonicalize)?;

        // TODO: replace with checks for manifest file once we transition
        if !env_path.join("flake.nix").exists() {
            Err(EnvironmentError2::DirectoryNotAnEnv)?
        }

        Ok(PathEnvironment {
            path: env_path,
            pointer,
            temp_dir: temp_dir.as_ref().to_path_buf(),
            state: Original,
        })
    }

    /// Create a new env in a `.flox` directory within a specific path or open it if it exists.
    ///
    /// The method creates or opens a `.flox` directory _contained_ within `path`!
    pub fn init(
        pointer: PathPointer,
        path: impl AsRef<Path>,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError2> {
        match EnvironmentPointer::open(path.as_ref()) {
            Err(EnvironmentError2::EnvNotFound) => {},
            Err(e) => Err(e)?,
            Ok(_) => Err(EnvironmentError2::EnvironmentExists)?,
        }

        let dot_flox_path = path.as_ref().join(DOT_FLOX);

        let env_dir = dot_flox_path.join(ENVIRONMENT_DIR_NAME);
        std::fs::create_dir_all(&env_dir).map_err(EnvironmentError2::InitEnv)?;

        let pointer_content =
            serde_json::to_string_pretty(&pointer).map_err(EnvironmentError2::SerializeEnvJson)?;

        let template_path = env!("FLOX_ENV_TEMPLATE");
        copy_dir_recursive(&template_path, &env_dir, false).map_err(EnvironmentError2::InitEnv)?;

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
    use crate::models::environment::MANIFEST_FILENAME;

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

        let expected = PathEnvironment {
            path: environment_temp_dir
                .path()
                .to_path_buf()
                .canonicalize()
                .unwrap()
                .join(".flox/env"),
            pointer: PathPointer::new("test".parse().unwrap()),
            temp_dir: temp_dir.path().to_path_buf(),
            state: Original,
        };

        let actual = PathEnvironment::<Original>::init(
            pointer,
            environment_temp_dir.into_path(),
            temp_dir.path(),
        )
        .unwrap();

        assert_eq!(actual, expected);

        assert!(actual.path.join("flake.nix").exists(), "flake exists");
        assert!(actual.manifest_path().exists(), "manifest exists");
        assert!(
            actual
                .path
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

        assert_eq!(temp_env.path.parent().unwrap(), sandbox_path);

        let new_env_str = r#"
        { }
        "#;

        temp_env.update_manifest(&new_env_str).unwrap();

        assert_eq!(temp_env.manifest_content().unwrap(), new_env_str);

        env.replace_with(temp_env).unwrap();

        assert_eq!(env.manifest_content().unwrap(), new_env_str);
    }

    #[tokio::test]
    #[cfg(feature = "impure-unit-tests")]
    #[ignore = "fixing in progress"]
    async fn test_install() {
        let (mut flox, tempdir) = flox_instance();
        flox.channels
            .register_channel("nixpkgs-flox", "github:flox/nixpkgs-flox".parse().unwrap());
        let (nix, system) = (flox.nix(Default::default()), flox.system);
        let pointer = PathPointer::new("test".parse().unwrap());

        let sandbox_path = tempdir.path().join("sandbox");
        std::fs::create_dir(&sandbox_path).unwrap();

        let mut env = PathEnvironment::init(pointer, tempdir.path(), &sandbox_path).unwrap();

        let mut temp_env = env.make_temporary().unwrap();

        let empty_env_str = r#"{ }"#;
        temp_env.update_manifest(&empty_env_str).unwrap();

        env.replace_with(temp_env).unwrap();

        env.install(
            [FloxPackage::Triple(FloxTriple {
                stability: Stability::Stable,
                channel: "nixpkgs-flox".to_string(),
                name: ["hello"].try_into().unwrap(),
                version: None,
            })]
            .to_vec(),
            &nix,
            system.clone(),
        )
        .await
        .unwrap();

        let installed_env_str = indoc! {r#"
            { packages."nixpkgs-flox".hello = { }; }
        "#};

        assert_eq!(env.manifest_content().unwrap(), installed_env_str);

        let catalog = env.catalog(&nix, system.clone()).await.unwrap();
        assert!(!catalog.entries.is_empty());

        assert!(env.out_link(&system).exists());
        assert!(env.catalog_path().exists());

        // Do a second install to make sure we can copy stuff like symlinks
        env.install(
            [FloxPackage::Triple(FloxTriple {
                stability: Stability::Stable,
                channel: "nixpkgs-flox".to_string(),
                name: ["curl"].try_into().unwrap(),
                version: None,
            })]
            .to_vec(),
            &nix,
            system.clone(),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    #[cfg(feature = "impure-unit-tests")]
    #[ignore = "fixing in progress"]
    async fn test_uninstall() {
        let (mut flox, tempdir) = flox_instance();
        flox.channels
            .register_channel("nixpkgs-flox", "github:flox/nixpkgs-flox".parse().unwrap());
        let (nix, system) = (flox.nix(Default::default()), flox.system);

        let pointer = PathPointer::new("test".parse().unwrap());

        let sandbox_path = tempdir.path().join("sandbox");
        std::fs::create_dir(&sandbox_path).unwrap();

        let mut env = PathEnvironment::init(pointer, tempdir.path(), &sandbox_path).unwrap();

        let mut temp_env = env.make_temporary().unwrap();

        let empty_env_str = indoc! {"
            { }
        "};

        temp_env.update_manifest(&empty_env_str).unwrap();

        env.replace_with(temp_env).unwrap();

        let package = FloxPackage::Triple(FloxTriple {
            stability: Stability::Stable,
            channel: "nixpkgs-flox".to_string(),
            name: ["hello"].try_into().unwrap(),
            version: None,
        });

        env.install([package.clone()].to_vec(), &nix, system.clone())
            .await
            .unwrap();

        let installed_env_str = indoc! {r#"
            { packages."nixpkgs-flox".hello = { }; }
        "#};

        assert_eq!(env.manifest_content().unwrap(), installed_env_str);

        let catalog = env.catalog(&nix, system.clone()).await.unwrap();
        assert!(!catalog.entries.is_empty());

        env.uninstall([package.clone()].to_vec(), &nix, system.clone())
            .await
            .unwrap();

        assert_eq!(env.manifest_content().unwrap(), empty_env_str);

        let catalog = env.catalog(&nix, system.clone()).await.unwrap();
        assert!(catalog.entries.is_empty());
    }
}
