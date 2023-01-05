use std::any::TypeId;
use std::collections::HashMap;
use std::env;
use std::str::FromStr;
use std::sync::Mutex;

use anyhow::Result;
use bpaf::{Bpaf, Parser};
use derive_more::{FromStr, Into};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::nix::arguments::flake::FlakeArgs;
use flox_rust_sdk::nix::arguments::NixArgs;
use flox_rust_sdk::nix::command::Eval;
use flox_rust_sdk::nix::command_line::{Group, NixCliCommand, NixCommandLine, ToArgs};
use flox_rust_sdk::nix::Run;
use flox_rust_sdk::prelude::Stability;
use once_cell::sync::Lazy;

use crate::config::{Config, Feature};
use crate::utils::InstallableDef;
use crate::{flox_forward, should_flox_forward, subcommand_metric};

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct BuildInstallable(String);
impl InstallableDef for BuildInstallable {
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] =
        &[("packages", true), ("legacyPackages", true)];
    const DERIVATION_TYPE: &'static str = "package";
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "build";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct DevelopInstallable(String);
impl InstallableDef for DevelopInstallable {
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] = &[
        ("packages", true),
        ("devShells", true),
        ("legacyPackages", true),
    ];
    const DERIVATION_TYPE: &'static str = "shell";
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "develop";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct PublishInstallable(String);
impl InstallableDef for PublishInstallable {
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] =
        &[("packages", true), ("legacyPackages", true)];
    const DERIVATION_TYPE: &'static str = "package";
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "publish";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct RunInstallable(String);
impl InstallableDef for RunInstallable {
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] =
        &[("packages", true), ("apps", true), ("legacyPackages", true)];
    const DERIVATION_TYPE: &'static str = "package";
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "build";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct ShellInstallable(String);
impl InstallableDef for ShellInstallable {
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] =
        &[("packages", true), ("legacyPackages", true)];
    const DERIVATION_TYPE: &'static str = "package";
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "shell";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct BundleInstallable(String);
impl InstallableDef for BundleInstallable {
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["."];
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] =
        &[("packages", true), ("legacyPackages", true)];
    const DERIVATION_TYPE: &'static str = "package";
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "bundle";
}

#[derive(FromStr, Default, Debug, Clone, Into)]
pub struct BundlerInstallable(String);
impl InstallableDef for BundlerInstallable {
    const ARG_FLAG: Option<&'static str> = Some("--bundler");
    const DEFAULT_FLAKEREFS: &'static [&'static str] = &["github:flox/bundlers/master"];
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)] = &[("bundlers", true)];
    const DERIVATION_TYPE: &'static str = "bundler";
    const INSTALLABLE: fn(&Self) -> String = |s| s.0.to_owned();
    const SUBCOMMAND: &'static str = "bundle";
}

#[allow(clippy::type_complexity)]
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

    /// parsed using [nix_arguments]
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
                subcommand_metric!("build");

                flox.package(
                    installable_arg.resolve_installable(&flox).await?,
                    package.stability(&config),
                    nix_arguments.clone(),
                )
                .build::<NixCommandLine>()
                .await?
            },

            PackageCommands::Develop {
                package: package @ PackageArgs { nix_arguments, .. },
                installable_arg,
                ..
            } => {
                subcommand_metric!("develop");

                flox.package(
                    installable_arg.resolve_installable(&flox).await?,
                    package.stability(&config),
                    nix_arguments.clone(),
                )
                .develop::<NixCommandLine>()
                .await?
            },
            PackageCommands::Run {
                package: package @ PackageArgs { nix_arguments, .. },
                installable_arg,
                ..
            } => {
                subcommand_metric!("run");

                flox.package(
                    installable_arg.resolve_installable(&flox).await?,
                    package.stability(&config),
                    nix_arguments.clone(),
                )
                .run::<NixCommandLine>()
                .await?
            },
            PackageCommands::Shell {
                package: package @ PackageArgs { nix_arguments, .. },
                installable_arg,
                ..
            } => {
                subcommand_metric!("shell");

                flox.package(
                    installable_arg.resolve_installable(&flox).await?,
                    package.stability(&config),
                    nix_arguments.clone(),
                )
                .shell::<NixCommandLine>()
                .await?
            },
            PackageCommands::Eval {
                package: package @ PackageArgs { nix_arguments, .. },
                ..
            } => {
                subcommand_metric!("eval");

                let nix = flox.nix::<NixCommandLine>(nix_arguments.clone());
                let command = Eval {
                    flake: FlakeArgs {
                        override_inputs: [package.stability(&config).as_override()].into(),
                        ..FlakeArgs::default()
                    },
                    ..Eval::default()
                };

                command.run(&nix, &NixArgs::default()).await?
            },
            PackageCommands::Bundle {
                package: package @ PackageArgs { nix_arguments, .. },
                installable_arg,
                bundler,
                ..
            } => {
                subcommand_metric!("bundle");

                flox.package(
                    installable_arg.resolve_installable(&flox).await?,
                    package.stability(&config),
                    nix_arguments.clone(),
                )
                .bundle::<NixCommandLine>(bundler.resolve_installable(&flox).await?)
                .await?
            },
            PackageCommands::Flake {
                subcommand,
                package,
            } => {
                // currently Flox::package requires _a package_.
                // since flake commands can't provide this flox.
                // we need to create a custom nix instance.
                // TODO: decide whether `flox flake` should be a "development command"
                //       It is currently implemented as such because it is influenced by `--stability`.
                //       Yet, it could be implemented as a different group altogether (more cleanly?).
                let nix: NixCommandLine = flox.nix(Default::default());

                // Flake commands should take `--stability`
                // Can't be a default on the `nix` instance, because that will apply it as a flag
                // on `nix flake` rather than `nix flake <subcommand>`.
                // Even though documented as "Common flake-related options",
                // flake args such as `--override-inputs` can not be applied to `nix flake`.
                // Inform [FlakeCommand] about the issued subcommand
                // and inject the flake args through its `ToArgs` implementation.
                FlakeCommand {
                    subcommand: subcommand.to_owned(),
                    default_flake_args: FlakeArgs {
                        override_inputs: [package.stability(&config).as_override()].into(),
                        ..Default::default()
                    },
                    args: package.nix_arguments.to_owned(),
                }
                .run(&nix, &Default::default())
                .await?;
            },

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

    /// run `nix flake` commands
    #[bpaf(command)]
    Flake {
        #[bpaf(positional("NIX FLAKE COMMAND"))]
        subcommand: String,

        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },
}

#[derive(Debug, Clone)]
pub struct FlakeCommand {
    subcommand: String,
    default_flake_args: FlakeArgs,
    args: Vec<String>,
}
impl ToArgs for FlakeCommand {
    fn to_args(&self) -> Vec<String> {
        let mut args = vec![self.subcommand.clone()];
        args.append(&mut self.default_flake_args.to_args());
        args.append(&mut self.args.clone());
        args
    }
}

impl NixCliCommand for FlakeCommand {
    type Own = Self;

    const FLAKE_ARGS: Group<Self, FlakeArgs> = Some(|_| Default::default());
    const OWN_ARGS: Group<Self, Self::Own> = Some(|s| s.to_owned());
    const SUBCOMMAND: &'static [&'static str] = &["flake"];
}
