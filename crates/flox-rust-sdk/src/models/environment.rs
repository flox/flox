use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use async_trait::async_trait;
use flox_types::catalog::{CatalogEntry, EnvCatalog};
use log::debug;
use rnix::ast::{AttrSet, Expr};
use rowan::ast::AstNode;
use runix::arguments::eval::EvaluationArgs;
use runix::arguments::{BuildArgs, EvalArgs};
use runix::command::{Build, Eval};
use runix::command_line::{NixCommandLine, NixCommandLineRunError, NixCommandLineRunJsonError};
use runix::flake_ref::path::PathRef;
use runix::installable::FlakeAttribute;
use runix::store_path::StorePath;
use runix::{Run, RunJson};
use thiserror::Error;
use walkdir::WalkDir;

use super::environment_ref::{EnvironmentName, EnvironmentRef, EnvironmentRefError};
use super::flox_package::{FloxPackage, FloxTriple};
use crate::utils::copy_file_without_permissions;
use crate::utils::rnix::{AttrSetExt, StrExt};

pub static CATALOG_JSON: &str = "catalog.json";
// don't forget to update the man page
pub const DEFAULT_KEEP_GENERATIONS: usize = 10;
// don't forget to update the man page
pub const DEFAULT_MAX_AGE_DAYS: u32 = 90;

pub enum InstalledPackage {
    Catalog(FloxTriple, CatalogEntry),
    FlakeAttribute(FlakeAttribute, CatalogEntry),
    StorePath(StorePath),
}

#[async_trait(?Send)]
pub trait Environment {
    type ConcreteTemporary;

    /// Build the environment and create a result link as gc-root
    async fn build(
        &self,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
    ) -> Result<(), EnvironmentError2>;

    /// Install packages to the environment atomically
    async fn install(
        &mut self,
        packages: impl IntoIterator<Item = FloxPackage>,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<&mut Self, EnvironmentError2>;

    /// Uninstall remove pacakges from the environment atomically
    async fn uninstall(
        &mut self,
        packages: impl IntoIterator<Item = FloxPackage>,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<&mut Self, EnvironmentError2>;

    /// Return the [EnvironmentRef] for the environment for identification
    fn environment_ref(&self) -> &EnvironmentRef;

    /// Read the catalog for this environment
    ///
    /// The catalog contains information about the locked sources of installed packages.
    async fn catalog(
        &self,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
    ) -> Result<EnvCatalog, EnvironmentError2>;

    /// List the installed packages
    async fn packages(
        &self,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
    ) -> Result<Vec<FloxPackage>, EnvironmentError2>;

    /// Create a temporary environment that can be modified freely
    async fn modify_in(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Self::ConcreteTemporary, EnvironmentError2>;

    /// Apply the changes made in a temporary environment
    fn replace_with(
        &mut self,
        temporary_environment: Self::ConcreteTemporary,
    ) -> Result<(), EnvironmentError2>;

    /// Delete the Environment
    fn delete(self) -> Result<(), EnvironmentError2>;
}

#[async_trait]
pub trait TemporaryEnvironment {
    async fn set_environment(
        &mut self,
        mut flox_nix_content: impl std::io::Read + Send,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<(), EnvironmentError2>;
}

/// Struct representing a local environment in a given location
#[derive(Debug)]
pub struct PathEnvironment<S> {
    /// absolute path to the environment, typically within `<...>/.flox/name`
    path: PathBuf,
    /// The [EnvironmentRef] this env is created from (and validated against)
    environment_ref: EnvironmentRef,
    state: S,
}

/// Changes to environments should be atomic,
/// so that possibly breaking modifications to the environment can be safely discarded.
/// When environment objects are created with [EnvironmentState] [Original] they
/// are in a "reading" state.
/// To make modifications we copy the environment into a temporary sandbox.
/// Within the sandbox we can make modifications and verify them by building the environment.
/// When verified, we can move the sandboxed environment back to its original location.
#[derive(Debug)]
pub struct Original {
    /// directory to create temporary environments in (usually flox.temp_dir)
    temp_dir: PathBuf,
}
pub struct Temporary;
pub trait EnvironmentState {}
impl EnvironmentState for Original {}
impl EnvironmentState for Temporary {}

impl<S> PartialEq for PathEnvironment<S> {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl<S: EnvironmentState> PathEnvironment<S> {
    /// Build the evironment derivation using nix
    async fn build(
        &self,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
    ) -> Result<(), EnvironmentError2> {
        debug!("building with nix ....");

        let build = Build {
            installables: [self.flake_attribute(system).into()].into(),
            eval: runix::arguments::eval::EvaluationArgs {
                impure: true.into(),
                ..Default::default()
            },
            build: BuildArgs {
                // out_link: Some(self.out_link().into()),
                ..Default::default()
            },
            ..Default::default()
        };

        build
            .run(nix, &Default::default())
            .await
            .map_err(EnvironmentError2::Build)?;

        Ok(())
    }
}

#[async_trait(?Send)]
impl Environment for PathEnvironment<Original> {
    type ConcreteTemporary = PathEnvironment<Temporary>;

    async fn build(
        &self,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
    ) -> Result<(), EnvironmentError2> {
        self.build(nix, system).await
    }

    /// Get the environment ref for this environment
    fn environment_ref(&self) -> &EnvironmentRef {
        &self.environment_ref
    }

    /// Install packages by converting a [FloxPackage]s into attributes in the `flox.nix` format,
    /// and then using [`rnix`](https://crates.io/crates/rnix) to merge these attributes into the
    /// environment definition file.
    async fn install(
        &mut self,
        packages: impl IntoIterator<Item = FloxPackage>,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<&mut Self, EnvironmentError2> {
        let flox_nix_content =
            fs::read_to_string(self.flox_nix_path()).map_err(EnvironmentError2::ReadFloxNix)?;
        let new_content = flox_nix_content_with_new_packages(&flox_nix_content, packages)?;

        let mut temporary_environment = self
            .modify_in(
                tempfile::tempdir_in(self.state.temp_dir.clone())
                    .unwrap()
                    .into_path(),
            )
            .await?;
        temporary_environment
            .set_environment(new_content.as_bytes(), nix, system)
            .await?;
        self.replace_with(temporary_environment)?;
        Ok(self)
    }

    /// Uninstall packages by converting a [FloxPackage]s into attributes in the `flox.nix` format,
    /// and then using [`rnix`](https://crates.io/crates/rnix) to remove these attributes from the
    /// environment definition file.
    async fn uninstall(
        &mut self,
        packages: impl IntoIterator<Item = FloxPackage>,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<&mut Self, EnvironmentError2> {
        let flox_nix_content =
            fs::read_to_string(self.flox_nix_path()).map_err(EnvironmentError2::ReadFloxNix)?;
        let new_content = flox_nix_content_with_packages_removed(&flox_nix_content, packages)?;

        let mut temporary_environment = self
            .modify_in(
                tempfile::tempdir_in(self.state.temp_dir.clone())
                    .unwrap()
                    .into_path(),
            )
            .await?;
        temporary_environment
            .set_environment(new_content.as_bytes(), nix, system)
            .await?;
        self.replace_with(temporary_environment)?;
        Ok(self)
    }

    /// Get a catalog of installed packages from the environment
    ///
    /// Evaluated using nix from the environment definition.
    async fn catalog(
        &self,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
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

        serde_json::from_value(catalog_value).map_err(EnvironmentError2::ParseCatalog)
    }

    /// List all Packages in the environment
    ///
    /// Currently unused, supposed to be a cleaned up version of [`Environment<_>::catalog`]
    async fn packages(
        &self,
        _nix: &NixCommandLine,
        _system: impl AsRef<str>,
    ) -> Result<Vec<FloxPackage>, EnvironmentError2> {
        todo!()
    }

    /// Copy the environment to a sandbox directory given by `path`
    ///
    /// Typically within [Flox.temp_dir].
    /// This implementation tries to stay independent of the [Flox] struct for now.
    async fn modify_in(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<PathEnvironment<Temporary>, EnvironmentError2> {
        copy_dir_recursively_without_permissions(&self.path, &path)
            .await
            .map_err(EnvironmentError2::MakeSandbox)?;

        Ok(PathEnvironment {
            path: path.as_ref().to_path_buf(),
            environment_ref: self.environment_ref.clone(),
            state: Temporary,
        })
    }

    /// Commmit changes, by moving modified files back to the original (read only) location
    fn replace_with(
        &mut self,
        temporary_environment: PathEnvironment<Temporary>,
    ) -> Result<(), EnvironmentError2> {
        fs_extra::dir::move_dir(
            &temporary_environment.path,
            &self.path,
            &fs_extra::dir::CopyOptions::new()
                .overwrite(true)
                .content_only(true),
        )
        .expect("replace origin");
        std::fs::create_dir(&temporary_environment.path).expect("recreate temp dir");
        Ok(())
    }

    /// Delete the environment
    ///
    /// While destructive, no transaction is needed to verify changes.
    fn delete(self) -> Result<(), EnvironmentError2> {
        std::fs::remove_dir_all(self.path).map_err(EnvironmentError2::DeleteEnvironement)?;
        Ok(())
    }
}

impl<S: EnvironmentState> PathEnvironment<S> {
    /// Remove gc-roots
    ///
    /// Currently stubbed out due to missing activation that could need linked results
    pub fn delete_symlinks(&self) -> Result<bool, EnvironmentError2> {
        // todo
        Ok(false)
    }

    /// Turn the environment into a flake attribute,
    /// a precise url to interact with the environment via nix
    ///
    /// ```
    /// # tokio_test::block_on(async {
    /// # use flox_rust_sdk::models::environment::{Original,PathEnvironment};
    /// # use std::path::PathBuf;
    /// # let tempdir = tempfile::tempdir().unwrap();
    /// # let environment_temp_dir = tempfile::tempdir().unwrap();
    /// # let path = tempdir.path().canonicalize().unwrap().to_string_lossy().into_owned();
    /// # let system = "aarch64-darwin";
    ///
    /// let env = PathEnvironment::<Original>::init(
    ///     &path,
    ///     "test_env".parse().unwrap(),
    ///     environment_temp_dir.into_path(),
    /// )
    /// .await
    /// .unwrap();
    ///
    /// let flake_attribute = format!("path:{path}/.flox/test_env#.floxEnvs.{system}.default")
    ///     .parse()
    ///     .unwrap();
    /// assert_eq!(env.flake_attribute(system), flake_attribute)
    ///
    /// # })
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
    pub fn flox_nix_path(&self) -> PathBuf {
        self.path.join("pkgs").join("default").join("flox.nix")
    }
}

/// Constructors of PathEnvironments
impl PathEnvironment<Original> {
    /// Open an environment at a given path
    ///
    /// Ensure that the path exists and contains files that "look" like an environment
    pub fn open(
        path: impl AsRef<Path>,
        ident: EnvironmentRef,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError2> {
        let path = path.as_ref().to_path_buf();
        let dot_flox_path = path.join(".flox");
        let env_path = dot_flox_path.join(ident.name().as_ref());

        if !env_path.exists() {
            Err(EnvironmentError2::EnvNotFound)?
        }

        if !env_path.is_dir() {
            Err(EnvironmentError2::EnvNotADirectory)?
        }

        let env_path = env_path
            .canonicalize()
            .map_err(EnvironmentError2::EnvCanonicalize)?;
        if !env_path.join("flake.nix").exists() {
            Err(EnvironmentError2::DirectoryNotAnEnv)?
        }

        Ok(PathEnvironment {
            path: env_path,
            environment_ref: ident,
            state: Original {
                temp_dir: temp_dir.as_ref().to_path_buf(),
            },
        })
    }

    /// Find the closest `.flox` starting with `current_dir`
    /// and looking up ancestor directories until `/`
    pub fn discover(
        current_dir: impl AsRef<Path>,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Option<Self>, EnvironmentError2> {
        let dot_flox = current_dir
            .as_ref()
            .ancestors()
            .find(|ancestor| ancestor.join(".flox").exists());

        let dot_flox = if let Some(dot_flox) = dot_flox {
            dot_flox
        } else {
            return Ok(None);
        };

        // assume only one entry in .flox
        let env = dot_flox
            .join(".flox")
            .read_dir()
            .map_err(EnvironmentError2::ReadDotFlox)?
            .next()
            .ok_or(EnvironmentError2::EmptyDotFlox)?
            .map_err(EnvironmentError2::ReadEnvDir)?;

        let name = EnvironmentName::from_str(&env.file_name().to_string_lossy())
            .map_err(EnvironmentError2::ParseEnvRef)?;

        Some(Self::open(
            current_dir,
            EnvironmentRef::new_from_parts(None, name),
            temp_dir,
        ))
        .transpose()
    }

    /// Create a new env in a `.flox` directory within a specific path or open it if it exists.
    ///
    /// The method creates or opens a `.flox` directory _contained_ within `path`!
    pub async fn init(
        path: impl AsRef<Path>,
        name: EnvironmentName,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError2> {
        if Self::open(
            &path,
            EnvironmentRef::new_from_parts(None, name.clone()),
            &temp_dir,
        )
        .is_ok()
        {
            Err(EnvironmentError2::EnvironmentExists)?;
        }

        let env_dir = path.as_ref().join(".flox").join(name.as_ref());

        std::fs::create_dir_all(&env_dir).map_err(EnvironmentError2::InitEnv)?;

        copy_dir_recursively_without_permissions(env!("FLOX_ENV_TEMPLATE"), &env_dir)
            .await
            .map_err(EnvironmentError2::InitEnv)?;

        Self::open(path, EnvironmentRef::new_from_parts(None, name), temp_dir)
    }
}

/// Implementations for environments in a "modifiable" state.
///
/// Created by [`PathEnvironment<Original>::modify_in`].
/// Allows editing the environment definition file.
#[async_trait]
impl TemporaryEnvironment for PathEnvironment<Temporary> {
    /// Low level method replacing the definition file content.
    /// After writing the file, we verify if by trying to build the environment.
    ///
    /// This might be deferred to the [`PathEnvironment<Original>::replace_with`] method in the future.
    async fn set_environment(
        &mut self,
        mut flox_nix_content: impl std::io::Read + Send,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<(), EnvironmentError2> {
        let mut flox_nix = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(self.flox_nix_path())
            .unwrap();
        std::io::copy(&mut flox_nix_content, &mut flox_nix).unwrap();
        self.build(nix, system).await?; // unwrap
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum EnvironmentError2 {
    #[error("ParseEnvRef")]
    ParseEnvRef(#[from] EnvironmentRefError),
    #[error("EmptyDotFlox")]
    EmptyDotFlox,
    #[error("DotFloxCanonicalize({0})")]
    EnvCanonicalize(std::io::Error),
    #[error("ReadDotFlox({0})")]
    ReadDotFlox(std::io::Error),
    #[error("ReadEnvDir({0})")]
    ReadEnvDir(std::io::Error),
    #[error("MakeSandbox({0})")]
    MakeSandbox(std::io::Error),
    #[error("DeleteEnvironment({0})")]
    DeleteEnvironement(std::io::Error),
    #[error("InitEnv({0})")]
    InitEnv(std::io::Error),
    #[error("EnvNotFound")]
    EnvNotFound,
    #[error("EnvNotADirectory")]
    EnvNotADirectory,
    #[error("DirectoryNotAnEnv")]
    DirectoryNotAnEnv,
    #[error("EnvironmentExists")]
    EnvironmentExists,
    #[error("EvalCatalog({0})")]
    EvalCatalog(NixCommandLineRunJsonError),
    #[error("ParseCatalog({0})")]
    ParseCatalog(serde_json::Error),
    #[error("Build({0})")]
    Build(NixCommandLineRunError),
    #[error("ReadFloxNix({0})")]
    ReadFloxNix(std::io::Error),
}

/// Within a nix AST, find the first definition of an attribute set,
/// that is not part of a `let` expression or a where clause
fn find_attrs(mut expr: Expr) -> Result<AttrSet, ()> {
    loop {
        match expr {
            Expr::LetIn(let_in) => expr = let_in.body().unwrap(),
            Expr::With(with) => expr = with.body().unwrap(),

            Expr::AttrSet(attrset) => return Ok(attrset),
            _ => return Err(()),
        }
    }
}

/// Copy a whole directory recursively ignoring the original permissions
async fn copy_dir_recursively_without_permissions(
    from: impl AsRef<Path>,
    to: &impl AsRef<Path>,
) -> Result<(), std::io::Error> {
    for entry in WalkDir::new(&from).into_iter().skip(1) {
        let entry = entry.unwrap();
        let new_path = to.as_ref().join(entry.path().strip_prefix(&from).unwrap());
        if entry.file_type().is_dir() {
            tokio::fs::create_dir(new_path).await.unwrap()
        } else {
            copy_file_without_permissions(entry.path(), &new_path)
                .await
                .unwrap()
        }
    }
    Ok(())
}

/// insert packages into the content of a flox.nix file
fn flox_nix_content_with_new_packages(
    flox_nix_content: &str,
    packages: impl IntoIterator<Item = FloxPackage>,
) -> Result<String, EnvironmentError2> {
    let packages = packages
        .into_iter()
        .map(|package| package.flox_nix_attribute().unwrap());

    let mut root = rnix::Root::parse(flox_nix_content)
        .ok()
        .unwrap()
        .expr()
        .unwrap();

    if let Expr::Lambda(lambda) = root {
        root = lambda.body().unwrap();
    }

    let config_attrset = find_attrs(root.clone()).unwrap();
    #[allow(clippy::redundant_clone)] // required for rnix reasons, i think
    let mut edited = config_attrset.clone();

    for (path, version) in packages {
        let mut value = rnix::ast::AttrSet::new();
        if let Some(version) = version {
            value = value.insert_unchecked(
                ["version"],
                rnix::ast::Str::new(&version).syntax().to_owned(),
            );
        }

        let mut path_in_packages = vec!["packages".to_string()];
        path_in_packages.extend_from_slice(&path);
        edited = edited.insert_unchecked(path_in_packages, value.syntax().to_owned());
    }

    let green_tree = config_attrset
        .syntax()
        .replace_with(edited.syntax().green().into_owned());
    let new_content = nixpkgs_fmt::reformat_string(&green_tree.to_string());
    Ok(new_content)
}

/// remove packages from the content of a flox.nix file
fn flox_nix_content_with_packages_removed(
    flox_nix_content: &str,
    packages: impl IntoIterator<Item = FloxPackage>,
) -> Result<String, EnvironmentError2> {
    let packages = packages
        .into_iter()
        .map(|package| package.flox_nix_attribute().unwrap());

    let mut root = rnix::Root::parse(flox_nix_content)
        .ok()
        .unwrap()
        .expr()
        .unwrap();

    if let Expr::Lambda(lambda) = root {
        root = lambda.body().unwrap();
    }

    let config_attrset = find_attrs(root.clone()).unwrap();

    #[allow(clippy::redundant_clone)] // required for rnix reasons, i think
    let mut edited = config_attrset.clone().syntax().green().into_owned();

    for (path, _version) in packages {
        let mut path_in_packages = vec!["packages".to_string()];
        path_in_packages.extend_from_slice(&path);

        let index = config_attrset
            .find_by_path(&path_in_packages)
            .unwrap_or_else(|| panic!("path not found, {path_in_packages:?}"))
            .syntax()
            .index();
        edited = edited.remove_child(index - 2); // yikes
    }

    let green_tree = config_attrset.syntax().replace_with(edited);
    let new_content = nixpkgs_fmt::reformat_string(&green_tree.to_string());
    Ok(new_content)
}

#[cfg(test)]
mod tests {
    use flox_types::stability::Stability;
    use indoc::indoc;

    use super::*;
    use crate::flox::tests::flox_instance;

    #[tokio::test]
    async fn create_env() {
        let tempdir = tempfile::tempdir().unwrap();
        let environment_temp_dir = tempfile::tempdir().unwrap();
        let before = PathEnvironment::<Original>::open(
            tempdir.path(),
            EnvironmentRef::new_from_parts(None, EnvironmentName::from_str("test").unwrap()),
            environment_temp_dir.path(),
        );

        assert!(
            matches!(before, Err(EnvironmentError2::EnvNotFound)),
            "{before:?}"
        );

        let expected = PathEnvironment {
            path: tempdir
                .path()
                .to_path_buf()
                .canonicalize()
                .unwrap()
                .join(".flox/test"),
            environment_ref: EnvironmentRef::new(None, "test").unwrap(),
            state: Original {
                temp_dir: environment_temp_dir.path().to_path_buf(),
            },
        };

        let actual = PathEnvironment::<Original>::init(
            tempdir.path(),
            EnvironmentName::from_str("test").unwrap(),
            environment_temp_dir.into_path(),
        )
        .await
        .unwrap();

        assert_eq!(actual, expected);

        assert!(actual.path.join("flake.nix").exists());
        assert!(actual
            .path
            .join("pkgs")
            .join("default")
            .join("flox.nix")
            .exists());
        assert!(actual
            .path
            .join("pkgs")
            .join("default")
            .join("default.nix")
            .exists());
        assert!(actual.path.is_absolute());
    }

    #[tokio::test]
    async fn flake_attribute() {
        let tempdir = tempfile::tempdir().unwrap();
        let environment_temp_dir = tempfile::tempdir().unwrap();
        let env = PathEnvironment::<Original>::init(
            tempdir.path(),
            "test".parse().unwrap(),
            environment_temp_dir.into_path(),
        )
        .await
        .unwrap();

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
        let (flox, tempdir) = flox_instance();
        let sandbox_path = tempdir.path().join("sandbox");
        std::fs::create_dir(&sandbox_path).unwrap();

        let mut env = PathEnvironment::init(tempdir.path(), "test".parse().unwrap(), &sandbox_path)
            .await
            .unwrap();

        let mut temp_env = env.modify_in(&sandbox_path).await.unwrap();

        assert_eq!(temp_env.path, sandbox_path);

        let new_env_str = r#"
        { }
        "#;

        temp_env
            .set_environment(
                new_env_str.as_bytes(),
                &flox.nix(Default::default()),
                flox.system,
            )
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(temp_env.flox_nix_path()).unwrap(),
            new_env_str
        );

        env.replace_with(temp_env).unwrap();

        assert_eq!(
            std::fs::read_to_string(env.flox_nix_path()).unwrap(),
            new_env_str
        );
    }

    #[tokio::test]
    #[cfg(feature = "impure-unit-tests")]
    async fn test_install() {
        let (mut flox, tempdir) = flox_instance();
        flox.channels
            .register_channel("nixpkgs-flox", "github:flox/nixpkgs-flox".parse().unwrap());
        let (nix, system) = (flox.nix(Default::default()), flox.system);

        let sandbox_path = tempdir.path().join("sandbox");
        std::fs::create_dir(&sandbox_path).unwrap();

        let mut env = PathEnvironment::init(tempdir.path(), "test".parse().unwrap(), &sandbox_path)
            .await
            .unwrap();

        let mut temp_env = env.modify_in(&sandbox_path).await.unwrap();

        let empty_env_str = r#"{ }"#;
        temp_env
            .set_environment(empty_env_str.as_bytes(), &nix, &system)
            .await
            .unwrap();

        env.replace_with(temp_env).unwrap();

        env.install(
            [FloxPackage::Triple(FloxTriple {
                stability: Stability::Stable,
                channel: "nixpkgs-flox".to_string(),
                name: ["hello"].try_into().unwrap(),
                version: None,
            })],
            &nix,
            &system,
        )
        .await
        .unwrap();

        let installed_env_str = indoc! {r#"
            { packages."nixpkgs-flox".hello = { }; }
        "#};

        assert_eq!(
            std::fs::read_to_string(env.flox_nix_path()).unwrap(),
            installed_env_str
        );

        let catalog = env.catalog(&nix, &system).await.unwrap();
        assert!(!catalog.entries.is_empty());
    }

    #[tokio::test]
    #[cfg(feature = "impure-unit-tests")]
    async fn test_uninstall() {
        let (mut flox, tempdir) = flox_instance();
        flox.channels
            .register_channel("nixpkgs-flox", "github:flox/nixpkgs-flox".parse().unwrap());
        let (nix, system) = (flox.nix(Default::default()), flox.system);

        let sandbox_path = tempdir.path().join("sandbox");
        std::fs::create_dir(&sandbox_path).unwrap();

        let mut env = PathEnvironment::init(tempdir.path(), "test".parse().unwrap(), &sandbox_path)
            .await
            .unwrap();

        let mut temp_env = env.modify_in(&sandbox_path).await.unwrap();

        let empty_env_str = indoc! {"
            { }
        "};

        temp_env
            .set_environment(empty_env_str.as_bytes(), &nix, &system)
            .await
            .unwrap();

        env.replace_with(temp_env).unwrap();

        let package = FloxPackage::Triple(FloxTriple {
            stability: Stability::Stable,
            channel: "nixpkgs-flox".to_string(),
            name: ["hello"].try_into().unwrap(),
            version: None,
        });

        env.install([package.clone()], &nix, &system).await.unwrap();

        let installed_env_str = indoc! {r#"
            { packages."nixpkgs-flox".hello = { }; }
        "#};

        assert_eq!(
            std::fs::read_to_string(env.flox_nix_path()).unwrap(),
            installed_env_str
        );

        let catalog = env.catalog(&nix, &system).await.unwrap();
        assert!(!catalog.entries.is_empty());

        env.uninstall([package.clone()], &nix, &system)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(env.flox_nix_path()).unwrap(),
            empty_env_str
        );

        let catalog = env.catalog(&nix, &system).await.unwrap();
        assert!(catalog.entries.is_empty());
    }
}
