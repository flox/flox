use std::ffi::OsStr;
use std::fs;
use std::io::Write;
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

use super::environment_ref::{
    EnvironmentName,
    EnvironmentOwner,
    EnvironmentRef,
    EnvironmentRefError,
};
use super::flox_package::{FloxPackage, FloxTriple};
use crate::utils::copy_file_without_permissions;
use crate::utils::errors::IoError;
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

#[async_trait]
pub trait Environment {
    /// Build the environment and create a result link as gc-root
    async fn build(
        &mut self,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<(), EnvironmentError2>;

    /// Install packages to the environment atomically
    async fn install(
        &mut self,
        packages: impl IntoIterator<Item = FloxPackage> + Send,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<bool, EnvironmentError2>;

    /// Uninstall packages from the environment atomically
    async fn uninstall(
        &mut self,
        packages: impl IntoIterator<Item = FloxPackage> + Send,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<(), EnvironmentError2>;

    /// Atomically edit this environment, ensuring that it still builds
    async fn edit(&mut self, contents: &impl AsRef<str>) -> Result<(), EnvironmentError2>;

    async fn catalog(
        &self,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<EnvCatalog, EnvironmentError2>;

    /// Return the [EnvironmentRef] for the environment for identification
    fn environment_ref(&self) -> &EnvironmentRef;

    /// Returns the environment owner
    fn owner(&self) -> Option<EnvironmentOwner>;

    /// Returns the environment name
    fn name(&self) -> EnvironmentName;

    /// Delete the Environment
    fn delete(self) -> Result<(), EnvironmentError2>;
}

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

    /// The [EnvironmentRef] this env is created from (and validated against)
    pub environment_ref: EnvironmentRef,

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
            environment_ref: self.environment_ref.clone(),
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
        fs::rename(replacement.path, &self.path).map_err(EnvironmentError2::Move)?;
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
            .join("builds")
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
        system: impl AsRef<str> + Send,
    ) -> Result<(), EnvironmentError2> {
        debug!("building with nix ....");

        let out_link = self.out_link(system.as_ref());

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
    async fn install(
        &mut self,
        packages: impl IntoIterator<Item = FloxPackage> + Send,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<bool, EnvironmentError2> {
        let current_manifest_contents =
            fs::read_to_string(self.manifest_path()).map_err(EnvironmentError2::ReadManifest)?;
        let new_manifest_contents =
            flox_nix_content_with_new_packages(&current_manifest_contents, packages)?;
        match new_manifest_contents {
            ManifestContent::Unchanged => return Ok(false),
            ManifestContent::Changed(new_manifest_contents) => {
                let mut temp_env = self.make_temporary()?;
                temp_env.update_manifest(&new_manifest_contents)?;
                temp_env.build(nix, system).await?;
                self.replace_with(temp_env)?;
                Ok(true)
            },
        }
    }

    /// Uninstall packages from the environment atomically
    async fn uninstall(
        &mut self,
        packages: impl IntoIterator<Item = FloxPackage> + Send,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
    ) -> Result<(), EnvironmentError2> {
        // We modify the contents of the manifest first so that someday in the future
        // when the `flox_nix_content...` function returns whether the file was actually
        // modified we can skip creating the temp dir when the file isn't modified.
        let current_manifest_contents =
            fs::read_to_string(self.manifest_path()).map_err(EnvironmentError2::ReadManifest)?;
        let new_manifest_contents =
            flox_nix_content_with_packages_removed(&current_manifest_contents, packages)?;
        let mut temp_env = self.make_temporary()?;
        temp_env.update_manifest(&new_manifest_contents)?;
        temp_env.build(nix, system).await?;
        self.replace_with(temp_env)?;
        Ok(())
    }

    #[allow(unused)]
    /// Atomically edit this environment, ensuring that it still builds
    async fn edit(&mut self, contents: &impl AsRef<str>) -> Result<(), EnvironmentError2> {
        todo!()
    }

    /// Return the [EnvironmentRef] for the environment for identification
    fn environment_ref(&self) -> &EnvironmentRef {
        &self.environment_ref
    }

    /// Get a catalog of installed packages from this environment
    ///
    /// Evaluated using nix from the environment definition.
    async fn catalog(
        &self,
        nix: &NixCommandLine,
        system: impl AsRef<str> + Send,
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

    /// Returns the environment owner
    fn owner(&self) -> Option<EnvironmentOwner> {
        self.environment_ref.owner().cloned()
    }

    /// Returns the environment name
    fn name(&self) -> EnvironmentName {
        self.environment_ref.name().clone()
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
}

impl<S: TransactionState> PathEnvironment<S> {
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
    pub fn manifest_path(&self) -> PathBuf {
        self.path.join("pkgs").join("default").join("flox.nix")
    }

    /// Path to the environment's catalog
    fn catalog_path(&self) -> PathBuf {
        self.path.join("pkgs").join("default").join(CATALOG_JSON)
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
            temp_dir: temp_dir.as_ref().to_path_buf(),
            state: Original,
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
    pub fn init(
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

        copy_dir_recursive(&env!("FLOX_ENV_TEMPLATE"), &env_dir, false)
            .map_err(EnvironmentError2::InitEnv)?;

        Self::open(path, EnvironmentRef::new_from_parts(None, name), temp_dir)
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
    DeleteEnvironment(std::io::Error),
    #[error("DotFloxNotFound")]
    DotFloxNotFound,
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
    #[error("WriteCatalog({0})")]
    WriteCatalog(std::io::Error),
    #[error("Build({0})")]
    Build(NixCommandLineRunError),
    #[error("ReadManifest({0})")]
    ReadManifest(std::io::Error),
    #[error("MakeTemporaryEnv({0})")]
    MakeTemporaryEnv(std::io::Error),
    #[error("UpdateManifest({0})")]
    UpdateManifest(std::io::Error),
    #[error("OpenManifest({0})")]
    OpenManifest(std::io::Error),
    #[error("Activate({0})")]
    Activate(NixCommandLineRunError),
    #[error("Prior transaction in progress. Delete {0} to discard.")]
    PriorTransaction(PathBuf),
    #[error("Failed to create backup for transaction: {0}")]
    BackupTransaction(std::io::Error),
    #[error("Failed to move modified environment into place: {0}")]
    Move(std::io::Error),
    #[error("Failed to remove transaction backup: {0}")]
    RemoveBackup(std::io::Error),
    #[error("Failed to copy file")]
    CopyFile(IoError),
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
///
/// We need this because:
/// 1. Sometimes we need to copy from the Nix store
/// 2. fs_extra::dir::copy doesn't handle symlinks.
///    See: https://github.com/webdesus/fs_extra/issues/61
fn copy_dir_recursive(
    from: &impl AsRef<Path>,
    to: &impl AsRef<Path>,
    keep_permissions: bool,
) -> Result<(), std::io::Error> {
    for entry in WalkDir::new(from).into_iter().skip(1) {
        let entry = entry.unwrap();
        let new_path = to.as_ref().join(entry.path().strip_prefix(from).unwrap());
        match entry.file_type() {
            file_type if file_type.is_dir() => {
                std::fs::create_dir(new_path).unwrap();
            },
            file_type if file_type.is_symlink() => {
                let target = std::fs::read_link(entry.path())
                // we know the path exists and is a symlink
                .unwrap();
                // If target is a relative symlink, this will potentially orphan
                // it. But we're assuming it's absolute since we only copy links
                // to the Nix store.
                std::os::unix::fs::symlink(target, &new_path)?;
                // TODO handle permissions
            },
            _ => {
                if keep_permissions {
                    fs::copy(entry.path(), &new_path)?;
                } else {
                    copy_file_without_permissions(entry.path(), &new_path).unwrap();
                }
            },
        }
    }
    Ok(())
}

pub enum ManifestContent {
    Unchanged,
    Changed(String),
}

/// insert packages into the content of a flox.nix file
///
/// TODO: At some point this should return None if the contents were not changed,
/// (e.g. the user tries to install a package that's already installed).
fn flox_nix_content_with_new_packages(
    flox_nix_content: &impl AsRef<str>,
    packages: impl IntoIterator<Item = FloxPackage>,
) -> Result<ManifestContent, EnvironmentError2> {
    let packages = packages
        .into_iter()
        .map(|package| package.flox_nix_attribute().unwrap());

    let mut root = rnix::Root::parse(flox_nix_content.as_ref())
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
    Ok(ManifestContent::Changed(new_content))
}

/// remove packages from the content of a flox.nix file
///
/// TODO: At some point this should indicate whether the contents were actually changed
/// (e.g. the user tries to install a package that's already installed).
fn flox_nix_content_with_packages_removed(
    flox_nix_content: &impl AsRef<str>,
    packages: impl IntoIterator<Item = FloxPackage>,
) -> Result<String, EnvironmentError2> {
    let packages = packages
        .into_iter()
        .map(|package| package.flox_nix_attribute().unwrap());

    let mut root = rnix::Root::parse(flox_nix_content.as_ref())
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
    #[cfg(feature = "impure-unit-tests")]
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
            temp_dir: environment_temp_dir.path().to_path_buf(),
            state: Original,
        };

        let actual = PathEnvironment::<Original>::init(
            tempdir.path(),
            EnvironmentName::from_str("test").unwrap(),
            environment_temp_dir.into_path(),
        )
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
        .unwrap();

        assert_eq!(
            env.flake_attribute("aarch64-darwin").to_string(),
            format!(
                "path:{}#.floxEnvs.aarch64-darwin.default",
                env.path.to_string_lossy()
            )
        )
    }

    #[test]
    fn test_flox_nix_content_with_new_packages() {
        let old_content = indoc! {r#"
            {
                packages."nixpkgs-flox".hello = {};
            }
        "#};
        let new_content =
            match flox_nix_content_with_new_packages(&old_content, [FloxPackage::Triple(
                FloxTriple {
                    stability: Stability::Stable,
                    channel: "nixpkgs-flox".to_string(),
                    name: ["hello"].try_into().unwrap(),
                    version: None,
                },
            )])
            .unwrap()
            {
                ManifestContent::Changed(new_content) => new_content,
                ManifestContent::Unchanged => panic!("contents should be changed"),
            };
        let expected = indoc! {r#"
            {
              packages."nixpkgs-flox".hello = { };
              packages."nixpkgs-flox".hello = { };
            }
        "#};
        pretty_assertions::assert_eq!(new_content, expected)
    }

    #[tokio::test]
    #[cfg(feature = "impure-unit-tests")]
    async fn edit_env() {
        let (_flox, tempdir) = flox_instance();
        let sandbox_path = tempdir.path().join("sandbox");
        std::fs::create_dir(&sandbox_path).unwrap();

        let mut env =
            PathEnvironment::init(tempdir.path(), "test".parse().unwrap(), &sandbox_path).unwrap();

        let mut temp_env = env.make_temporary().unwrap();

        assert_eq!(temp_env.path.parent().unwrap(), sandbox_path);

        let new_env_str = r#"
        { }
        "#;

        temp_env.update_manifest(&new_env_str).unwrap();

        assert_eq!(
            std::fs::read_to_string(temp_env.manifest_path()).unwrap(),
            new_env_str
        );

        env.replace_with(temp_env).unwrap();

        assert_eq!(
            std::fs::read_to_string(env.manifest_path()).unwrap(),
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

        let mut env =
            PathEnvironment::init(tempdir.path(), "test".parse().unwrap(), &sandbox_path).unwrap();

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
            std::fs::read_to_string(env.manifest_path()).unwrap(),
            installed_env_str
        );

        let catalog = env.catalog(&nix, &system).await.unwrap();
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
            })],
            &nix,
            &system,
        )
        .await
        .unwrap();
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

        let mut env =
            PathEnvironment::init(tempdir.path(), "test".parse().unwrap(), &sandbox_path).unwrap();

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

        env.install([package.clone()], &nix, &system).await.unwrap();

        let installed_env_str = indoc! {r#"
            { packages."nixpkgs-flox".hello = { }; }
        "#};

        assert_eq!(
            std::fs::read_to_string(env.manifest_path()).unwrap(),
            installed_env_str
        );

        let catalog = env.catalog(&nix, &system).await.unwrap();
        assert!(!catalog.entries.is_empty());

        env.uninstall([package.clone()], &nix, &system)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(env.manifest_path()).unwrap(),
            empty_env_str
        );

        let catalog = env.catalog(&nix, &system).await.unwrap();
        assert!(catalog.entries.is_empty());
    }
}
