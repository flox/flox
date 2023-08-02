use std::path::{Path, PathBuf};

use flox_types::catalog::{CatalogEntry, EnvCatalog};
use itertools::Itertools;
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
    Installable(FlakeAttribute, CatalogEntry),
    StorePath(StorePath),
}

#[derive(Debug)]
pub struct Environment<S> {
    path: PathBuf,
    state: S,
}

impl<S: State> State for Environment<S> {
    fn owner(&self) -> Option<&str> {
        self.state.owner()
    }

    fn name(&self) -> &str {
        self.state.name()
    }
}

impl<S> PartialEq for Environment<S> {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl<S: State> Environment<S> {
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
        }
    }

    #[allow(dead_code)] // used later
    fn out_link(&self) -> PathBuf {
        self.path.join("gc-root")
    }

    // todo: do a nix build
    //       how / when to set gc root?
    async fn build(
        &self,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
    ) -> Result<(), EnvironmentError2> {
        println!("building with nix ....");

        let build = Build {
            installables: [self.flake_attribute(system).into()].into(),
            eval: runix::arguments::eval::EvaluationArgs {
                impure: true.into(),
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

    pub fn flox_nix_path(&self) -> PathBuf {
        self.path.join("pkgs").join("default").join("flox.nix")
    }

    pub async fn catalog(
        &self,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
    ) -> Result<EnvCatalog, EnvironmentError2> {
        let mut flake_attribute = self.flake_attribute(system);
        flake_attribute.attr_path.push_attr("catalog").unwrap(); // valid attribute name, should not fail

        let eval = Eval {
            eval: EvaluationArgs {
                impure: true.into(),
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

    pub async fn packages(
        &self,
        _nix: &NixCommandLine,
        _system: impl AsRef<str>,
    ) -> Result<Vec<FloxPackage>, EnvironmentError2> {
        todo!()
    }

    pub fn delete_symlinks(&self) -> Result<bool, EnvironmentError2> {
        // todo
        Ok(false)
    }
}

impl Environment<Read> {
    fn new(
        owner: Option<String>,
        name: String,
        path: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError2> {
        let path = path.as_ref().to_path_buf();

        if !path.exists() {
            Err(EnvironmentError2::EnvNotFound)?
        }
        if !path.is_dir() {
            Err(EnvironmentError2::EnvNotADirectory)?
        }
        if !path.join("flake.nix").exists() {
            Err(EnvironmentError2::DirectoryNotAnEnv)?
        }

        Ok(Self {
            path,
            state: Read { owner, name },
        })
    }

    pub async fn modify_in(
        self,
        path: impl AsRef<Path>,
    ) -> Result<Environment<Modify>, EnvironmentError2> {
        cp_r(&self.path, &path)
            .await
            .map_err(EnvironmentError2::MakeSandbox)?;

        Ok(Environment {
            path: path.as_ref().to_path_buf(),
            state: Modify { origin: self },
        })
    }
}

impl Environment<Modify> {
    pub async fn set_environment(
        &mut self,
        mut flox_nix_content: impl std::io::Read,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
    ) -> Result<&mut Self, EnvironmentError2> {
        let mut flox_nix = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(self.flox_nix_path())
            .unwrap();
        std::io::copy(&mut flox_nix_content, &mut flox_nix).unwrap();
        self.build(nix, system).await?; // unwrap
        Ok(self)
    }

    pub async fn install(
        &mut self,
        packages: impl IntoIterator<Item = FloxPackage>,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
    ) -> Result<&mut Self, EnvironmentError2> {
        let packages = packages
            .into_iter()
            .map(|package| package.flox_nix_attribute().unwrap());

        // assume flake exists locally
        let flox_nix_path = self.flox_nix_path();
        let flox_nix_content: String = std::fs::read_to_string(&flox_nix_path).unwrap();

        let mut root = rnix::Root::parse(&flox_nix_content)
            .ok()
            .unwrap()
            .expr()
            .unwrap();

        if let Expr::Lambda(lambda) = root {
            root = lambda.body().unwrap();
        }

        let config_attrset = find_attrs(root.clone()).unwrap();
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

        self.set_environment(new_content.as_bytes(), nix, system)
            .await
    }

    pub async fn uninstall(
        &mut self,
        packages: impl IntoIterator<Item = FloxPackage>,
        nix: &NixCommandLine,
        system: impl AsRef<str>,
    ) -> Result<&mut Self, EnvironmentError2> {
        let packages = packages
            .into_iter()
            .map(|package| package.flox_nix_attribute().unwrap());

        // assume flake exists locally
        let flox_nix_path = self.flox_nix_path();
        let flox_nix_content: String = std::fs::read_to_string(&flox_nix_path).unwrap();

        let mut root = rnix::Root::parse(&flox_nix_content)
            .ok()
            .unwrap()
            .expr()
            .unwrap();

        if let Expr::Lambda(lambda) = root {
            root = lambda.body().unwrap();
        }

        let config_attrset = find_attrs(root.clone()).unwrap();
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

        self.set_environment(dbg!(new_content).as_bytes(), nix, system)
            .await
    }

    pub fn finish(self) -> Result<Environment<Read>, EnvironmentError2> {
        fs_extra::dir::move_dir(
            &self.path,
            &self.state.origin.path,
            &fs_extra::dir::CopyOptions::new()
                .overwrite(true)
                .content_only(true),
        )
        .expect("replace origin");
        Ok(self.state.origin)
    }
}

pub trait State {
    fn owner(&self) -> Option<&str>;
    fn name(&self) -> &str;
}

#[derive(Debug, PartialEq)]
pub struct Read {
    owner: Option<String>,
    name: String,
}

impl State for Read {
    fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug)]
pub struct Modify {
    origin: Environment<Read>,
}

impl State for Modify {
    fn owner(&self) -> Option<&str> {
        self.origin.state.owner()
    }

    fn name(&self) -> &str {
        self.origin.state.name()
    }
}

pub struct DotFloxDir {
    path: PathBuf,
}

impl DotFloxDir {
    pub fn discover(current_dir: impl AsRef<Path>) -> Result<Self, EnvironmentError2> {
        let dot_flox = current_dir
            .as_ref()
            .ancestors()
            .find(|ancestor| ancestor.join(".flox").exists())
            .ok_or(EnvironmentError2::NoDotFloxFound)?;

        Self::open(dot_flox)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, EnvironmentError2> {
        let dot_flox = path.as_ref().join(".flox");
        if !dot_flox.exists() {
            Err(EnvironmentError2::NoDotFloxFound)?
        }
        if !dot_flox.is_dir() {
            Err(EnvironmentError2::DotFloxNotADirectory)?
        }

        let path = dot_flox
            .canonicalize()
            .map_err(EnvironmentError2::DotFloxCanonicalize)?;

        Ok(Self { path })
    }

    pub fn new(path: impl AsRef<Path>) -> Result<DotFloxDir, EnvironmentError2> {
        std::fs::create_dir(path.as_ref().join(".flox")).expect("create dot_flox dir");
        Self::open(path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn environments(&self) -> Result<Vec<Environment<Read>>, EnvironmentError2> {
        self.path
            .read_dir()
            .map_err(EnvironmentError2::ReadDotFlox)?
            .map(|env_or_user| {
                let env_or_user = env_or_user.map_err(EnvironmentError2::ReadDotFlox)?;
                let envs = match self.environment(None, env_or_user.file_name().to_string_lossy()) {
                    Ok(env) => vec![env],
                    Err(EnvironmentError2::DirectoryNotAnEnv) => env_or_user
                        .path()
                        .read_dir()
                        .map_err(EnvironmentError2::ReadOwnerDir)?
                        .map(|env| {
                            let env = env.map_err(EnvironmentError2::ReadDotFlox)?;
                            self.environment(
                                Some(env_or_user.file_name().to_string_lossy().to_string()),
                                env.file_name().to_string_lossy(),
                            )
                        })
                        .collect::<Result<Vec<Environment<Read>>, EnvironmentError2>>()?,
                    Err(e) => Err(e)?,
                };
                Ok(envs)
            })
            .flatten_ok()
            .collect()
    }

    pub fn environment(
        &self,
        owner: Option<String>,
        name: impl AsRef<str>,
    ) -> Result<Environment<Read>, EnvironmentError2> {
        let mut path = self.path.clone();
        if let Some(ref owner) = owner.as_ref() {
            path = path.join(owner);
        }
        path = path.join(name.as_ref());
        Environment::new(
            owner.as_ref().map(ToString::to_string),
            name.as_ref().to_string(),
            path,
        )
    }

    pub async fn create_env(
        &mut self,
        name: impl AsRef<str>,
    ) -> Result<Environment<Read>, EnvironmentError2> {
        if name.as_ref().chars().any(|c| ['/', '~'].contains(&c)) {
            Err(EnvironmentError2::InvalidName)?
        }

        if self.environment(None, &name).is_ok() {
            Err(EnvironmentError2::EnvironmentExists)?
        }

        let env_dir = self.path.join(name.as_ref());

        std::fs::create_dir(self.path.join(name.as_ref())).expect("create env dir");

        cp_r(env!("FLOX_ENV_TEMPLATE"), &env_dir)
            .await
            .expect("copy template");

        Environment::new(None, name.as_ref().to_string(), env_dir)
    }

    #[allow(dead_code)]
    fn pull_env(
        _owner: impl AsRef<str>,
        _name: impl AsRef<str>,
        _auth: (),
    ) -> Result<Environment<Read>, EnvironmentError2> {
        todo!() // figure out pulling in next phase
    }
}

#[derive(Debug, Error)]
pub enum EnvironmentError2 {
    #[error("NoDotFloxFound")]
    NoDotFloxFound,
    #[error("DotFloxNotADirectory")]
    DotFloxNotADirectory,
    #[error("DotFloxCanonicalize({0})")]
    DotFloxCanonicalize(std::io::Error),
    #[error("ReadDotFlox({0})")]
    ReadDotFlox(std::io::Error),
    #[error("ReadOwnerDir({0})")]
    ReadOwnerDir(std::io::Error),
    #[error("ReadEnvDir({0})")]
    ReadEnvDir(std::io::Error),
    #[error("MakeSandbox({0})")]
    MakeSandbox(std::io::Error),

    #[error("EnvNotFound")]
    EnvNotFound,
    #[error("EnvNotADirectory")]
    EnvNotADirectory,
    #[error("DirectoryNotAnEnv")]
    DirectoryNotAnEnv,

    #[error("EnvironmentExists")]
    EnvironmentExists,
    #[error("InvalidName")]
    InvalidName,
    #[error("EvalCatalog({0})")]
    EvalCatalog(NixCommandLineRunJsonError),
    #[error("ParseCatalog({0})")]
    ParseCatalog(serde_json::Error),
    #[error("Build({0})")]
    Build(NixCommandLineRunError),
}

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

async fn cp_r(from: impl AsRef<Path>, to: &impl AsRef<Path>) -> Result<(), std::io::Error> {
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

#[cfg(test)]
mod tests {
    use flox_types::stability::Stability;
    use indoc::indoc;

    use super::*;
    use crate::flox::tests::flox_instance;

    #[test]
    fn create_dot_flox() {
        let tempdir = tempfile::tempdir().unwrap();
        let _dot_flox_dir = DotFloxDir::new(tempdir.path()).unwrap();
        assert!(tempdir.path().join(".flox").exists());
    }

    #[tokio::test]
    async fn create_env() {
        let tempdir = tempfile::tempdir().unwrap();
        let mut dot_flox_dir = DotFloxDir::new(tempdir.path()).unwrap();

        assert_eq!(&dot_flox_dir.environments().unwrap(), &[]);

        assert!(matches!(
            dot_flox_dir.create_env("test/bla").await,
            Err(EnvironmentError2::InvalidName)
        ));

        let expected = Environment {
            path: dot_flox_dir.path.join("test"),
            state: Read {
                name: "test".to_string(),
                owner: None,
            },
        };

        let actual = dot_flox_dir.create_env("test").await.unwrap();

        assert_eq!(actual, expected);
        assert_eq!(actual.state, expected.state);

        assert_eq!(&dot_flox_dir.environments().unwrap(), &[expected]);

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
        let mut dot_flox_dir = DotFloxDir::new(tempdir.path()).unwrap();
        let env = dot_flox_dir.create_env("test").await.unwrap();

        assert_eq!(
            env.flake_attribute("aarch64-darwin").to_string(),
            format!(
                "path:{}#.floxEnvs.aarch64-darwin.default",
                env.path.to_string_lossy()
            )
        )
    }

    #[tokio::test]
    async fn edit_env() {
        let (flox, tempdir) = flox_instance();
        let sandbox_path = tempdir.path().join("sandbox");
        std::fs::create_dir(&sandbox_path).unwrap();

        let mut dot_flox_dir = DotFloxDir::new(tempdir.path()).unwrap();
        let env = dot_flox_dir.create_env("test").await.unwrap();
        let mut env = env.modify_in(&sandbox_path).await.unwrap();

        assert_eq!(env.path, sandbox_path);

        let new_env_str = r#"
        { }
        "#;

        env.set_environment(
            new_env_str.as_bytes(),
            &flox.nix(Default::default()),
            flox.system,
        )
        .await
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(env.flox_nix_path()).unwrap(),
            new_env_str
        );

        let env = env.finish().unwrap();

        assert_eq!(
            std::fs::read_to_string(env.flox_nix_path()).unwrap(),
            new_env_str
        );
    }

    #[tokio::test]
    async fn test_install() {
        let (mut flox, tempdir) = flox_instance();
        flox.channels
            .register_channel("nixpkgs-flox", "github:flox/nixpkgs-flox".parse().unwrap());
        let (nix, system) = (flox.nix(Default::default()), flox.system);

        let sandbox_path = tempdir.path().join("sandbox");
        std::fs::create_dir(&sandbox_path).unwrap();

        let mut dot_flox_dir = DotFloxDir::new(tempdir.path()).unwrap();
        let env = dot_flox_dir.create_env("test").await.unwrap();
        let mut env = env.modify_in(&sandbox_path).await.unwrap();

        let empty_env_str = r#"{ }"#;
        env.set_environment(empty_env_str.as_bytes(), &nix, &system)
            .await
            .unwrap();

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

        let env = env.finish().unwrap();

        assert_eq!(
            std::fs::read_to_string(env.flox_nix_path()).unwrap(),
            installed_env_str
        );

        let catalog = env.catalog(&nix, &system).await.unwrap();
        assert!(!catalog.entries.is_empty());
    }

    #[tokio::test]
    async fn test_uninstall() {
        let (mut flox, tempdir) = flox_instance();
        flox.channels
            .register_channel("nixpkgs-flox", "github:flox/nixpkgs-flox".parse().unwrap());
        let (nix, system) = (flox.nix(Default::default()), flox.system);

        let sandbox_path = tempdir.path().join("sandbox");
        std::fs::create_dir(&sandbox_path).unwrap();

        let mut dot_flox_dir = DotFloxDir::new(tempdir.path()).unwrap();
        let env = dot_flox_dir.create_env("test").await.unwrap();
        let mut env = env.modify_in(&sandbox_path).await.unwrap();

        let empty_env_str = indoc! {"
            { }
        "};

        env.set_environment(empty_env_str.as_bytes(), &nix, &system)
            .await
            .unwrap();

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

        let env: Environment<Read> = env.finish().unwrap();

        assert_eq!(
            std::fs::read_to_string(env.flox_nix_path()).unwrap(),
            empty_env_str
        );

        let catalog = env.catalog(&nix, &system).await.unwrap();
        assert!(catalog.entries.is_empty());
    }
}
