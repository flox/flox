use std::env;
use std::fmt::Debug;

use anyhow::Result;
use bpaf::{construct, Bpaf, Parser};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::nix::arguments::flake::FlakeArgs;
use flox_rust_sdk::nix::arguments::NixArgs;
use flox_rust_sdk::nix::command::Eval as EvalComm;
use flox_rust_sdk::nix::command_line::{Group, NixCliCommand, NixCommandLine, ToArgs};
use flox_rust_sdk::nix::Run as RunC;
use flox_rust_sdk::prelude::Stability;

use crate::config::features::Feature;
use crate::config::Config;
use crate::{flox_forward, subcommand_metric};

pub(crate) mod interface {
    use bpaf::{Bpaf, Parser};

    use super::parseable_macro::parseable;
    use super::{package_args, PackageArgs, Parseable, WithPassthru};
    use crate::utils::installables::{
        BuildInstallable,
        BundleInstallable,
        BundlerInstallable,
        DevelopInstallable,
        PublishInstallable,
        RunInstallable,
        ShellInstallable,
    };
    use crate::utils::{InstallableArgument, Parsed};

    #[derive(Debug, Clone, Bpaf)]
    pub struct Build {
        #[bpaf(external(package_args), group_help("Development Options"))]
        pub(crate) package: PackageArgs,

        #[bpaf(short('A'), hide)]
        pub(crate) _attr_flag: bool,

        #[bpaf(external(InstallableArgument::positional))]
        pub(crate) installable_arg: Option<InstallableArgument<Parsed, BuildInstallable>>,
    }
    parseable!(Build, build);

    #[derive(Bpaf, Clone, Debug)]
    /// launch development shell for current project
    pub struct Develop {
        #[bpaf(external(package_args), group_help("Development Options"))]
        pub package: PackageArgs,

        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,

        /// Shell or package to develop on
        #[bpaf(external(InstallableArgument::positional))]
        pub(crate) installable_arg: Option<InstallableArgument<Parsed, DevelopInstallable>>,
    }
    parseable!(Develop, develop);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Publish {
        #[bpaf(external(package_args), group_help("Development Options"))]
        pub package: PackageArgs,

        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,

        /// The --channel-repo determines the upstream repository containing
        #[bpaf(argument("REPO"))]
        pub channel_repo: String,

        #[bpaf(argument("REPO"))]
        pub build_repo: String,

        #[bpaf(argument("URL"))]
        pub upload_to: String,

        #[bpaf(argument("URL"))]
        pub download_from: String,

        #[bpaf(argument("DIR"))]
        pub render_path: String,

        #[bpaf(argument("FILE"))]
        pub key_file: String,

        #[bpaf(argument("FILE"))]
        pub publish_system: String,

        /// Package to publish
        #[bpaf(external(InstallableArgument::positional))]
        pub(crate) _installable_arg: Option<InstallableArgument<Parsed, PublishInstallable>>,
    }
    parseable!(Publish, publish);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Shell {
        #[bpaf(external(package_args), group_help("Development Options"))]
        pub package: PackageArgs,

        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,

        /// Package to provide in a shell
        #[bpaf(external(InstallableArgument::positional))]
        pub(crate) installable_arg: Option<InstallableArgument<Parsed, ShellInstallable>>,
    }
    parseable!(Shell, shell);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Bundle {
        #[bpaf(external(package_args), group_help("Development Options"))]
        pub package: PackageArgs,

        /// Bundler to use
        #[bpaf(external)]
        pub(crate) bundler_arg: Option<InstallableArgument<Parsed, BundlerInstallable>>,

        /// Package to bundle
        #[bpaf(external(InstallableArgument::positional))]
        pub(crate) installable_arg: Option<InstallableArgument<Parsed, BundleInstallable>>,

        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,
    }
    parseable!(Bundle, bundle);
    pub(crate) fn bundler_arg(
    ) -> impl Parser<Option<InstallableArgument<Parsed, BundlerInstallable>>> {
        InstallableArgument::parse_with(bpaf::long("bundler").short('b').argument("bundler"))
    }

    #[derive(Bpaf, Clone, Debug)]
    pub struct Run {
        #[bpaf(external(package_args), group_help("Development Options"))]
        pub(crate) package: PackageArgs,

        #[bpaf(short('A'), hide)]
        pub(crate) _attr_flag: bool,
        #[bpaf(external(InstallableArgument::positional))]
        pub(crate) installable_arg: Option<InstallableArgument<Parsed, RunInstallable>>,
    }
    parseable!(Run, run);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Eval {
        #[bpaf(external(package_args), group_help("Development Options"))]
        pub(crate) package: PackageArgs,
    }
    parseable!(Eval, eval);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Flake {
        #[bpaf(external(package_args), group_help("Development Options"))]
        pub package: PackageArgs,

        #[bpaf(positional("NIX FLAKE COMMAND"))]
        pub subcommand: String,
    }
    parseable!(Flake, flake);

    #[derive(Bpaf, Clone, Debug)]
    pub enum PackageCommands {
        /// initialize flox expressions for current project
        #[bpaf(command)]
        Init {},
        /// build package from current project
        #[bpaf(command)]
        Build(#[bpaf(external(WithPassthru::parse))] WithPassthru<Build>),
        /// launch development shell for current project
        #[bpaf(command)]
        Develop(#[bpaf(external(WithPassthru::parse))] WithPassthru<Develop>),
        /// build and publish project to flox channel
        #[bpaf(command)]
        Publish(#[bpaf(external(WithPassthru::parse))] WithPassthru<Publish>),
        /// run app from current project
        #[bpaf(command)]
        Run(#[bpaf(external(WithPassthru::parse))] WithPassthru<Run>),
        /// run a shell in which the current project is available
        #[bpaf(command)]
        Shell(#[bpaf(external(WithPassthru::parse))] WithPassthru<Shell>),
        /// evaluate a Nix expression
        #[bpaf(command)]
        Eval(#[bpaf(external(WithPassthru::parse))] WithPassthru<Eval>),
        /// run a bundler for current project
        #[bpaf(command)]
        Bundle(#[bpaf(external(WithPassthru::parse))] WithPassthru<Bundle>),
        /// run `nix flake` commands
        #[bpaf(command)]
        Flake(#[bpaf(external(WithPassthru::parse))] WithPassthru<Flake>),
    }
}

impl interface::PackageCommands {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            _ if Feature::Nix.is_forwarded()? => flox_forward(&flox).await?,

            // Unification implemntation of Develop is not yet implmented in rust
            interface::PackageCommands::Develop(_) if Feature::Develop.is_forwarded()? => {
                flox_forward(&flox).await?
            },

            // `flox publish` is not yet implmented in rust
            interface::PackageCommands::Publish(_) if Feature::Publish.is_forwarded()? => {
                flox_forward(&flox).await?
            },

            interface::PackageCommands::Build(command) => {
                subcommand_metric!("build");
                let installable_arg = command
                    .inner
                    .installable_arg
                    .unwrap_or_default()
                    .resolve_installable(&flox)
                    .await?;

                flox.package(
                    installable_arg,
                    command.inner.package.stability(&config),
                    command.nix_args,
                )
                .build::<NixCommandLine>()
                .await?
            },
            interface::PackageCommands::Develop(command) => {
                subcommand_metric!("develop");

                let installable_arg = command
                    .inner
                    .installable_arg
                    .unwrap_or_default()
                    .resolve_installable(&flox)
                    .await?;

                flox.package(
                    installable_arg,
                    command.inner.package.stability(&config),
                    command.nix_args,
                )
                .develop::<NixCommandLine>()
                .await?
            },
            interface::PackageCommands::Run(command) => {
                subcommand_metric!("run");

                let installable_arg = command
                    .inner
                    .installable_arg
                    .unwrap_or_default()
                    .resolve_installable(&flox)
                    .await?;

                flox.package(
                    installable_arg,
                    command.inner.package.stability(&config),
                    command.nix_args,
                )
                .run::<NixCommandLine>()
                .await?
            },
            interface::PackageCommands::Shell(command) => {
                subcommand_metric!("shell");

                let installable_arg = command
                    .inner
                    .installable_arg
                    .unwrap_or_default()
                    .resolve_installable(&flox)
                    .await?;

                flox.package(
                    installable_arg,
                    command.inner.package.stability(&config),
                    command.nix_args,
                )
                .shell::<NixCommandLine>()
                .await?
            },
            interface::PackageCommands::Eval(command) => {
                subcommand_metric!("eval");

                let nix = flox.nix::<NixCommandLine>(command.nix_args);
                let command = EvalComm {
                    flake: FlakeArgs {
                        override_inputs: [command.inner.package.stability(&config).as_override()]
                            .into(),
                        ..FlakeArgs::default()
                    },
                    ..Default::default()
                };

                command.run(&nix, &NixArgs::default()).await?
            },
            interface::PackageCommands::Bundle(command) => {
                subcommand_metric!("bundle");

                let installable_arg = command
                    .inner
                    .installable_arg
                    .unwrap_or_default()
                    .resolve_installable(&flox)
                    .await?;

                let bundler = command
                    .inner
                    .bundler_arg
                    .unwrap_or_default()
                    .resolve_installable(&flox)
                    .await?;

                flox.package(
                    installable_arg,
                    command.inner.package.stability(&config),
                    command.nix_args,
                )
                .bundle::<NixCommandLine>(bundler)
                .await?
            },
            interface::PackageCommands::Flake(command) => {
                /// A custom nix command that passes its arguments to `nix flake`
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
                    subcommand: command.inner.subcommand.to_owned(),
                    default_flake_args: FlakeArgs {
                        override_inputs: [command.inner.package.stability(&config).as_override()]
                            .into(),
                        ..Default::default()
                    },
                    args: command.nix_args,
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
pub struct PackageArgs {
    #[bpaf(long, argument("STABILITY"))]
    stability: Option<Stability>,
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

#[derive(Debug, Clone)]
pub struct WithPassthru<T> {
    inner: T,
    nix_args: Vec<String>,
}

impl<T> WithPassthru<T> {
    fn with_parser(inner: impl Parser<T>) -> impl Parser<Self> {
        let nix_args = bpaf::positional("args")
            .strict()
            .many()
            .anywhere()
            .fallback(Default::default())
            .hide();

        let fake_args = bpaf::any("args")
            .guard(
                |m: &String| !["--help", "-h"].contains(&m.as_str()),
                "asdas",
            )
            // .strict()
            .many();

        construct!(nix_args, inner, fake_args).map(|(mut nix_args, inner, mut fake_args)| {
            // dbg!(&nix_args, &inner, &fake_args);

            nix_args.append(&mut fake_args);

            WithPassthru { inner, nix_args }
        })
    }
}

pub trait Parseable: Sized {
    fn parse() -> bpaf::parsers::ParseBox<Self>;
}

impl<T: Parseable + Debug + 'static> Parseable for WithPassthru<T> {
    fn parse() -> bpaf::parsers::ParseBox<WithPassthru<T>> {
        let parser = WithPassthru::with_parser(T::parse());
        construct!(parser)
    }
}

mod parseable_macro {

    /// This macro takes a type
    /// and implmenets the [Parseable] trait for it
    /// using the specified bpaf parser.
    /// Intended to be used with parser function generated by bpaf.
    /// As a trait method this allows for more convenience when deriving parsers.
    macro_rules! parseable {
        ($type:ty, $parser:ident) => {
            impl crate::commands::package::Parseable for $type {
                fn parse() -> bpaf::parsers::ParseBox<Self> {
                    let p = $parser();
                    bpaf::construct!(p)
                }
            }
        };
    }
    pub(crate) use parseable;
}
