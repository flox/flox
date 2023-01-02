use std::{any::TypeId, collections::HashMap, env, str::FromStr, sync::Mutex};

use anyhow::Result;
use bpaf::{Bpaf, Parser};
use derive_more::{FromStr, Into};
use flox_rust_sdk::{
    flox::Flox,
    nix::{
        arguments::{flake::FlakeArgs, NixArgs},
        command::Eval,
        command_line::NixCommandLine,
        Run, RunJson,
    },
    prelude::Stability,
};
use once_cell::sync::Lazy;

use crate::{
    config::{Config, Feature},
    flox_forward, should_flox_forward,
    utils::{metrics::metric, InstallableDef},
};

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct BuildInstallable(String);
impl InstallableDef for BuildInstallable {
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] =
        &[("packages", true), ("legacyPackages", true)];
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "build";
    const DERIVATION_TYPE: &'static str = "package";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct DevelopInstallable(String);
impl InstallableDef for DevelopInstallable {
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] = &[
        ("packages", true),
        ("devShells", true),
        ("legacyPackages", true),
    ];
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "develop";
    const DERIVATION_TYPE: &'static str = "shell";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct PublishInstallable(String);
impl InstallableDef for PublishInstallable {
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] =
        &[("packages", true), ("legacyPackages", true)];
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "publish";
    const DERIVATION_TYPE: &'static str = "package";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct RunInstallable(String);
impl InstallableDef for RunInstallable {
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] =
        &[("packages", true), ("apps", true), ("legacyPackages", true)];
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "build";
    const DERIVATION_TYPE: &'static str = "package";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct ShellInstallable(String);
impl InstallableDef for ShellInstallable {
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] =
        &[("packages", true), ("legacyPackages", true)];
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "shell";
    const DERIVATION_TYPE: &'static str = "package";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct BundleInstallable(String);
impl InstallableDef for BundleInstallable {
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] =
        &[("packages", true), ("legacyPackages", true)];
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "bundle";
    const DERIVATION_TYPE: &'static str = "package";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct BundlerInstallable(String);
impl InstallableDef for BundlerInstallable {
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] = &[("bundlers", true)];
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["github:flox/bundlers/master"];
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "bundle";
    const DERIVATION_TYPE: &'static str = "bundler";
    const ARG_FLAG: Option<&'static str> = Some("--bundler");
}

static COMPLETED_INSTALLABLES: Lazy<
    Mutex<HashMap<(TypeId, String), Vec<(String, Option<String>)>>>,
> = Lazy::new(|| Mutex::new(HashMap::new()));

fn complete_installable<T: InstallableDef + 'static>(
    inst_arg: &T,
) -> Vec<(String, Option<String>)> {
    COMPLETED_INSTALLABLES
        .lock()
        .unwrap()
        .entry((TypeId::of::<T>(), T::INSTALLABLE(inst_arg)))
        .or_insert_with(|| inst_arg.complete_inst())
        .to_vec()
}

#[derive(Bpaf, Clone, Debug)]
pub struct PackageArgs {
    #[bpaf(long, argument("STABILITY"), optional)]
    stability: Option<Stability>,

    #[bpaf(external)]
    nix_arguments: Vec<String>,
}

impl PackageArgs {
    /// Resolve stability from flag or config (which reads environment variables).
    /// If the stability is set by a flag, modify STABILITY env variable to match
    /// the set stability.
    /// Flox invocations in a child process will inherit hence inherit the stability.
    fn stability(&self, config: &Config) -> Stability {
        if let Some(ref stability) = self.stability {
            env::set_var("FLOX_PREVIEW_STABILITY", stability.to_string());
            stability.clone()
        } else {
            config.flox.stability.clone()
        }
    }
}

fn installable_arg<T>() -> impl Parser<T>
where
    T: InstallableDef + 'static,
    <T as FromStr>::Err: std::fmt::Display,
{
    bpaf::positional("INSTALLABLE")
        .complete(complete_installable)
        .fallback(Default::default())
        .adjacent()
}

fn nix_arguments() -> impl Parser<Vec<String>> {
    extra_args("NIX ARGUMENTS")
}

fn extra_args(var: &'static str) -> impl Parser<Vec<String>> {
    bpaf::any(var)
        .guard(
            |m: &String| !["--help", "-h"].contains(&m.as_str()),
            "Not A Nix Arg",
        )
        .many()
}

impl PackageCommands {
    pub async fn handle(&self, config: Config, flox: Flox) -> Result<()> {
        match self {
            _ if should_flox_forward(Feature::Nix)? => flox_forward(&flox).await?,

            PackageCommands::Build {
                package: package @ PackageArgs { nix_arguments, .. },
                installable_arg,
                ..
            } => {
                metric("build");

                flox.package(
                    installable_arg.resolve_installable(&flox).await?,
                    package.stability(&config),
                    nix_arguments.clone(),
                )
                .build::<NixCommandLine>()
                .await?
            }

            PackageCommands::Develop {
                package: package @ PackageArgs { nix_arguments, .. },
                installable_arg,
                ..
            } => {
                metric("develop");

                flox.package(
                    installable_arg.resolve_installable(&flox).await?,
                    package.stability(&config),
                    nix_arguments.clone(),
                )
                .develop::<NixCommandLine>()
                .await?
            }
            PackageCommands::Run {
                package: package @ PackageArgs { nix_arguments, .. },
                installable_arg,
                ..
            } => {
                metric("run");

                flox.package(
                    installable_arg.resolve_installable(&flox).await?,
                    package.stability(&config),
                    nix_arguments.clone(),
                )
                .run::<NixCommandLine>()
                .await?
            }
            PackageCommands::Shell {
                package: package @ PackageArgs { nix_arguments, .. },
                installable_arg,
                ..
            } => {
                metric("shell");

                flox.package(
                    installable_arg.resolve_installable(&flox).await?,
                    package.stability(&config),
                    nix_arguments.clone(),
                )
                .shell::<NixCommandLine>()
                .await?
            }
            PackageCommands::Eval {
                package: package @ PackageArgs { nix_arguments, .. },
                ..
            } => {
                let nix = flox.nix::<NixCommandLine>(nix_arguments.clone());
                let command = Eval {
                    flake: FlakeArgs {
                        override_inputs: [package.stability(&config).as_override()].into(),
                        ..FlakeArgs::default()
                    },
                    ..Eval::default()
                };

                command.run(&nix, &NixArgs::default()).await?
            }
            PackageCommands::Bundle {
                package: package @ PackageArgs { nix_arguments, .. },
                installable_arg,
                bundler,
                ..
            } => {
                flox.package(
                    installable_arg.resolve_installable(&flox).await?,
                    package.stability(&config),
                    nix_arguments.clone(),
                )
                .bundle::<NixCommandLine>(bundler.resolve_installable(&flox).await?)
                .await?
            }
            _ => todo!(),
        }

        Ok(())
    }
}

#[derive(Bpaf, Clone, Debug)]
pub enum PackageCommands {
    /// initialize flox expressions for current project
    #[bpaf(command)]
    Init {},

    /// build package from current project
    #[bpaf(command)]
    Build {
        #[bpaf(short('A'), hide)]
        _attr_flag: bool,
        #[bpaf(external)]
        installable_arg: BuildInstallable,

        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },

    /// launch development shell for current project
    #[bpaf(command)]
    Develop {
        #[bpaf(short('A'), hide)]
        _attr_flag: bool,
        #[bpaf(external)]
        installable_arg: DevelopInstallable,

        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },
    /// build and publish project to flox channel
    #[bpaf(command)]
    Publish {
        #[bpaf(short('A'), hide)]
        _attr_flag: bool,
        #[bpaf(external)]
        installable_arg: PublishInstallable,

        /// The --channel-repo determines the upstream repository containing
        #[bpaf(argument("REPO"))]
        channel_repo: String,

        #[bpaf(argument("REPO"))]
        build_repo: String,

        #[bpaf(argument("URL"))]
        upload_to: String,

        #[bpaf(argument("URL"))]
        download_from: String,

        #[bpaf(argument("DIR"))]
        render_path: String,

        #[bpaf(argument("FILE"))]
        key_file: String,

        #[bpaf(argument("FILE"))]
        publish_system: String,

        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },
    /// run app from current project
    #[bpaf(command)]
    Run {
        #[bpaf(short('A'), hide)]
        _attr_flag: bool,
        #[bpaf(external)]
        installable_arg: RunInstallable,

        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },
    /// run a shell in which the current project is available
    #[bpaf(command)]
    Shell {
        #[bpaf(short('A'), hide)]
        _attr_flag: bool,
        #[bpaf(external)]
        installable_arg: ShellInstallable,

        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },

    /// evaluate a Nix expression
    #[bpaf(command)]
    Eval {
        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },

    /// run a bundler for current project
    #[bpaf(command)]
    Bundle {
        #[bpaf(
            short,
            long,
            complete(complete_installable),
            fallback(Default::default())
        )]
        bundler: BundlerInstallable,

        #[bpaf(short('A'), hide)]
        _attr_flag: bool,
        #[bpaf(external)]
        installable_arg: BundleInstallable,

        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },
}
