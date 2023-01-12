use std::env;
use std::fmt::Debug;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use bpaf::{construct, Bpaf, Parser};
use flox_rust_sdk::actions::project::{self, Closed, Open, Project};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::nix::arguments::flake::FlakeArgs;
use flox_rust_sdk::nix::arguments::NixArgs;
use flox_rust_sdk::nix::command::Eval as EvalComm;
use flox_rust_sdk::nix::command_line::{Group, NixCliCommand, NixCommandLine, ToArgs};
use flox_rust_sdk::nix::Run as RunC;
use flox_rust_sdk::prelude::Stability;
use flox_rust_sdk::providers::git::GitCommandProvider;
use futures::TryFutureExt;
use inquire::error::InquireResult;
use log::info;

use crate::commands::package::interface::ResolveInstallable;
use crate::config::features::Feature;
use crate::config::Config;
use crate::utils::dialog::{Confirm, Dialog, Text};
use crate::{flox_forward, subcommand_metric};

pub(crate) mod interface {
    use async_trait::async_trait;
    use bpaf::{Bpaf, Parser};
    use flox_rust_sdk::flox::{EnvironmentRef, Flox};
    use flox_rust_sdk::prelude::Installable;
    use flox_rust_sdk::providers::git::GitProvider;

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
        TemplateInstallable,
    };
    use crate::utils::{InstallableArgument, InstallableDef, Parsed};

    /// TODO:
    /// I would like a `-e` flag with no following argument to default to `PosOrEnv::Env("")`,
    /// and no arguments to default to `PosOrEnv::Pos(InstallableArgument<Parsed, T>::default())`
    #[derive(Clone, Debug)]
    pub enum PosOrEnv<T: InstallableDef> {
        Pos(InstallableArgument<Parsed, T>),
        Env(String),
    }
    impl<T: 'static + InstallableDef> PosOrEnv<T> {
        fn parser() -> impl Parser<Option<PosOrEnv<T>>> {
            let p = InstallableArgument::positional();
            let e = bpaf::long("environment")
                .short('e')
                .argument("environment")
                .optional();

            let pp = p.map(|x| x.map(PosOrEnv::Pos));
            let ee = e.map(|x| x.map(PosOrEnv::Env));

            bpaf::construct!([pp, ee])
        }
    }

    #[async_trait(?Send)]
    pub trait ResolveInstallable<Git: GitProvider> {
        async fn installable(&self, flox: &Flox) -> anyhow::Result<Installable>;
    }

    #[async_trait(?Send)]
    impl<T: InstallableDef + 'static, Git: GitProvider> ResolveInstallable<Git> for PosOrEnv<T> {
        async fn installable(&self, flox: &Flox) -> anyhow::Result<Installable> {
            Ok(match self {
                PosOrEnv::Pos(i) => i.resolve_installable(flox).await?,
                PosOrEnv::Env(n) => {
                    EnvironmentRef::new::<Git>(flox, n)
                        .await?
                        .get_latest_installable(flox)
                        .await?
                },
            })
        }
    }

    #[async_trait(?Send)]
    impl<T: InstallableDef + 'static, Git: GitProvider> ResolveInstallable<Git>
        for Option<PosOrEnv<T>>
    {
        async fn installable(&self, flox: &Flox) -> anyhow::Result<Installable> {
            Ok(match self {
                Some(x) => ResolveInstallable::<Git>::installable(x, flox).await?,
                None => {
                    ResolveInstallable::<Git>::installable(
                        &PosOrEnv::Pos(InstallableArgument::<Parsed, T>::default()),
                        flox,
                    )
                    .await?
                },
            })
        }
    }

    #[derive(Debug, Clone, Bpaf)]
    pub struct Init {
        // [sic]
        // template does NOT support package args
        // - e.g. `stability`
        #[bpaf(external(template_arg))]
        pub(crate) template: Option<InstallableArgument<Parsed, TemplateInstallable>>,
        #[bpaf(long("name"), short('n'), argument("name"))]
        pub(crate) name: Option<String>,
        #[bpaf(short('i'), long("init-git"), switch)]
        pub(crate) init_git: bool,
    }
    pub(crate) fn template_arg(
    ) -> impl Parser<Option<InstallableArgument<Parsed, TemplateInstallable>>> {
        InstallableArgument::parse_with(bpaf::long("template").short('t').argument("template"))
    }

    parseable!(Init, init);

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
    /// print shell code that can be sourced by bash to reproduce the development environment
    pub struct PrintDevEnv {
        #[bpaf(external(package_args), group_help("Development Options"))]
        pub package: PackageArgs,

        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,

        /// Shell or package to develop on
        #[bpaf(external(InstallableArgument::positional))]
        pub(crate) _installable_arg: Option<InstallableArgument<Parsed, DevelopInstallable>>,
    }
    parseable!(PrintDevEnv, print_dev_env);

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

        /// Package or environment to bundle
        #[bpaf(external(PosOrEnv::parser))]
        pub(crate) installable_arg: Option<PosOrEnv<BundleInstallable>>,

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
        Init(#[bpaf(external(WithPassthru::parse))] WithPassthru<Init>),
        /// build package from current project
        #[bpaf(command)]
        Build(#[bpaf(external(WithPassthru::parse))] WithPassthru<Build>),
        /// launch development shell for current project
        #[bpaf(command)]
        Develop(#[bpaf(external(WithPassthru::parse))] WithPassthru<Develop>),
        /// print shell code that can be sourced by bash to reproduce the development environment
        #[bpaf(command("print-dev-env"))]
        PrintDevEnv(#[bpaf(external(WithPassthru::parse))] WithPassthru<PrintDevEnv>),
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

            // Unification implemntation of print-dev-env is not yet implmented in rust
            interface::PackageCommands::PrintDevEnv(_) if Feature::Develop.is_forwarded()? => {
                flox_forward(&flox).await?
            },

            // `flox publish` is not yet implmented in rust
            interface::PackageCommands::Publish(_) if Feature::Publish.is_forwarded()? => {
                flox_forward(&flox).await?
            },

            interface::PackageCommands::Init(command) => {
                subcommand_metric!("init");

                let cwd = std::env::current_dir()?;
                let basename = cwd
                    .file_name()
                    .and_then(|x| x.to_str())
                    .unwrap_or("NAME")
                    .to_owned();

                let git_repo = ensure_project_repo(&flox, cwd, &command).await?;
                let project = ensure_project(git_repo, &command).await?;

                let name = match command.inner.name {
                    Some(n) => n,
                    None => {
                        let dialog = Dialog {
                            message: "Enter package name",
                            help_message: None,
                            typed: Text {
                                default: Some(&basename),
                            },
                        };

                        dialog.prompt().await.context("Failed to prompt for name")?
                    },
                };

                let name = name.trim();

                if !name.is_empty() {
                    let template = command
                        .inner
                        .template
                        .unwrap_or_default()
                        .resolve_installable(&flox)
                        .await?;

                    project
                        .init_flox_package::<NixCommandLine>(command.nix_args, template, name)
                        .await?;
                }
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

                let installable_arg = ResolveInstallable::<GitCommandProvider>::installable(
                    &command.inner.installable_arg,
                    &flox,
                )
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

async fn ensure_project_repo<'flox>(
    flox: &'flox Flox,
    cwd: PathBuf,
    command: &WithPassthru<interface::Init>,
) -> Result<project::Project<'flox, Closed<GitCommandProvider>>, anyhow::Error> {
    let git_repo = flox
        .project(cwd)
        .guard::<GitCommandProvider>()
        .await?
        .open()
        .inspect_ok(|p| {
            info!(
                "Found git repo{}",
                p.workdir()
                    .map(|p| format!(": {}", p.display()))
                    .unwrap_or_else(|| "".to_owned())
            );
        })
        .or_else(|g| async move {
            async fn prompt(workdir: Option<&Path>) -> InquireResult<bool> {
                let in_text = workdir.map(|x| format!("in {}", x.display())).unwrap_or_else(|| "here".to_owned());

                let dialog = Dialog {
                    message: &format!("The current directory is not in a Git repository, would you like to create one {}?", in_text),
                    help_message: None,
                    typed: Confirm {
                        default: Some(false)
                    }
                };

                dialog.prompt().await
            }

            if command.inner.init_git || prompt(g.workdir()).await? {
                let p = g.init_git().await?;

                info!(
                    "Created git repo{}",
                    p.workdir()
                        .map(|p| format!(": {}", p.display()))
                        .unwrap_or_else(|| "".to_owned())
                );

                Ok(p)
            } else {
                bail!("You must be inside of a Git repository to initialize a project");
            }
        })
        .await?;
    Ok(git_repo)
}

/// Create
async fn ensure_project<'flox>(
    git_repo: Project<'flox, Closed<GitCommandProvider>>,
    command: &WithPassthru<interface::Init>,
) -> Result<Project<'flox, Open<GitCommandProvider>>> {
    let project = git_repo
        .guard()
        .await?
        .open()
        .or_else(|g| g.init_project::<NixCommandLine>(command.nix_args.clone()))
        .await?;
    Ok(project)
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
