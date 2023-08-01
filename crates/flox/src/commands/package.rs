use std::env;
use std::fmt::Debug;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use bpaf::{construct, Bpaf, Parser};
use crossterm::tty::IsTty;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::project::Project;
use flox_rust_sdk::models::publish::{Publish, PublishFlakeRef};
use flox_rust_sdk::models::root::transaction::ReadOnly;
use flox_rust_sdk::models::root::{self, Closed, Root};
use flox_rust_sdk::nix::arguments::eval::EvaluationArgs;
use flox_rust_sdk::nix::arguments::flake::FlakeArgs;
use flox_rust_sdk::nix::arguments::NixArgs;
use flox_rust_sdk::nix::command::{Build, BuildOut, Eval as EvalComm};
use flox_rust_sdk::nix::command_line::{Group, NixCliCommand, NixCommandLine, ToArgs};
use flox_rust_sdk::nix::{Run as RunC, RunTyped};
use flox_rust_sdk::prelude::FlakeAttribute;
use flox_rust_sdk::providers::git::{GitCommandProvider, GitProvider};
use flox_types::stability::Stability;
use indoc::indoc;
use itertools::Itertools;
use log::{debug, info};

use crate::commands::package::interface::{PackageCommands, ResolveInstallable};
use crate::config::features::Feature;
use crate::config::Config;
use crate::utils::dialog::{Dialog, Text};
use crate::utils::resolve_environment_ref;
use crate::{flox_forward, subcommand_metric};

async fn env_ref_to_flake_attribute<Git: GitProvider + 'static>(
    flox: &Flox,
    subcommand: &str,
    environment_name: &str,
) -> anyhow::Result<FlakeAttribute> {
    let env_ref = resolve_environment_ref::<Git>(flox, subcommand, Some(environment_name)).await?;
    Ok(env_ref.get_latest_flake_attribute::<Git>(flox).await?)
}

pub(crate) mod interface {
    use async_trait::async_trait;
    use bpaf::{Bpaf, Parser};
    use flox_rust_sdk::flox::Flox;
    use flox_rust_sdk::prelude::FlakeAttribute;
    use flox_rust_sdk::providers::git::GitProvider;

    use super::parseable_macro::parseable;
    use super::{env_ref_to_flake_attribute, Parseable, WithPassthru};
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

    #[derive(Clone, Debug)]
    pub enum PosOrEnv<T: InstallableDef> {
        Pos(InstallableArgument<Parsed, T>),
        Env(String),
    }
    impl<T: 'static + InstallableDef> Parseable for PosOrEnv<T> {
        fn parse() -> bpaf::parsers::ParseBox<Self> {
            let installable = InstallableArgument::positional().map(PosOrEnv::Pos);
            let environment = bpaf::long("environment")
                .short('e')
                .argument("environment")
                .map(PosOrEnv::Env);

            let parser = bpaf::construct!([installable, environment]);
            bpaf::construct!(parser) // turn into a box
        }
    }

    #[async_trait(?Send)]
    pub trait ResolveInstallable<Git: GitProvider> {
        async fn installable(&self, flox: &Flox) -> anyhow::Result<FlakeAttribute>;
    }

    #[async_trait(?Send)]
    impl<T: InstallableDef + 'static, Git: GitProvider + 'static> ResolveInstallable<Git>
        for PosOrEnv<T>
    {
        async fn installable(&self, flox: &Flox) -> anyhow::Result<FlakeAttribute> {
            Ok(match self {
                PosOrEnv::Pos(i) => i.resolve_flake_attribute(flox).await?,
                PosOrEnv::Env(n) => {
                    env_ref_to_flake_attribute::<Git>(flox, T::SUBCOMMAND, n).await?
                },
            })
        }
    }

    #[async_trait(?Send)]
    impl<T: InstallableDef + 'static, Git: GitProvider + 'static> ResolveInstallable<Git>
        for Option<PosOrEnv<T>>
    {
        async fn installable(&self, flox: &Flox) -> anyhow::Result<FlakeAttribute> {
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
    pub struct Nix {}

    #[derive(Debug, Clone, Bpaf)]
    pub struct Init {
        // [sic]
        // template does NOT support package args
        // - e.g. `stability`
        #[bpaf(external(template_arg))]
        pub(crate) template: Option<InstallableArgument<Parsed, TemplateInstallable>>,
        #[bpaf(long("name"), short('n'), argument("name"))]
        pub(crate) name: Option<String>,
    }
    pub(crate) fn template_arg(
    ) -> impl Parser<Option<InstallableArgument<Parsed, TemplateInstallable>>> {
        InstallableArgument::parse_with(bpaf::long("template").short('t').argument("template"))
            .optional()
    }

    parseable!(Init, init);

    #[derive(Debug, Clone, Bpaf)]
    pub struct Build {
        #[bpaf(short('A'), hide)]
        pub(crate) _attr_flag: bool,

        #[bpaf(external(InstallableArgument::positional), optional, catch)]
        pub(crate) installable_arg: Option<InstallableArgument<Parsed, BuildInstallable>>,
    }
    parseable!(Build, build);

    #[derive(Bpaf, Clone, Debug)]
    /// launch development shell for current project
    pub struct Develop {
        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,

        /// Shell or package to develop on
        #[bpaf(external(InstallableArgument::positional), optional, catch)]
        pub(crate) installable_arg: Option<InstallableArgument<Parsed, DevelopInstallable>>,
    }
    parseable!(Develop, develop);

    #[derive(Bpaf, Clone, Debug)]
    /// print shell code that can be sourced by bash to reproduce the development environment
    pub struct PrintDevEnv {
        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,

        /// Shell or package to develop on
        #[bpaf(external(InstallableArgument::positional), optional, catch)]
        pub(crate) _installable_arg: Option<InstallableArgument<Parsed, DevelopInstallable>>,
    }
    parseable!(PrintDevEnv, print_dev_env);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Publish {
        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,

        /// The --channel-repo determines the upstream repository containing
        #[bpaf(argument("REPO"))]
        pub channel_repo: Option<String>,

        #[bpaf(argument("REPO"))]
        pub build_repo: Option<String>,

        #[bpaf(argument("URL"))]
        pub upload_to: Option<String>,

        #[bpaf(argument("URL"))]
        pub download_from: Option<String>,

        #[bpaf(argument("DIR"))]
        pub render_path: Option<String>,

        #[bpaf(argument("FILE"))]
        pub key_file: Option<String>,

        #[bpaf(argument("FILE"))]
        pub publish_system: Option<String>,

        /// Package to publish
        #[bpaf(external(InstallableArgument::positional), optional, catch)]
        pub(crate) _installable_arg: Option<InstallableArgument<Parsed, PublishInstallable>>,
    }
    parseable!(Publish, publish);

    #[derive(Bpaf, Clone, Debug)]
    pub struct PublishV2 {
        /// Package to publish
        #[bpaf(external(InstallableArgument::positional), optional, catch)]
        pub installable_arg: Option<InstallableArgument<Parsed, PublishInstallable>>,
    }
    parseable!(PublishV2, publish_v2);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Shell {
        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,

        /// Package to provide in a shell
        #[bpaf(external(InstallableArgument::positional), optional, catch)]
        pub(crate) installable_arg: Option<InstallableArgument<Parsed, ShellInstallable>>,
    }
    parseable!(Shell, shell);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Bundle {
        /// Bundler to use
        #[bpaf(external)]
        pub(crate) bundler_arg: Option<InstallableArgument<Parsed, BundlerInstallable>>,

        /// Package or environment to bundle
        #[bpaf(external(PosOrEnv::parse), optional, catch)]
        pub(crate) installable_arg: Option<PosOrEnv<BundleInstallable>>,

        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,
    }
    parseable!(Bundle, bundle);
    pub(crate) fn bundler_arg(
    ) -> impl Parser<Option<InstallableArgument<Parsed, BundlerInstallable>>> {
        InstallableArgument::parse_with(bpaf::long("bundler").short('b').argument("bundler"))
            .optional()
    }

    #[derive(Bpaf, Clone, Debug)]
    pub struct Containerize {
        /// Environment to containerize
        #[bpaf(long("environment"), short('e'), argument("ENV"))]
        pub(crate) environment_name: Option<String>,

        #[bpaf(short('A'), hide)]
        pub _attr_flag: bool,
    }
    parseable!(Containerize, containerize);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Run {
        #[bpaf(short('A'), hide)]
        pub(crate) _attr_flag: bool,
        #[bpaf(external(InstallableArgument::positional), optional, catch)]
        pub(crate) installable_arg: Option<InstallableArgument<Parsed, RunInstallable>>,
    }
    parseable!(Run, run);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Eval {}
    parseable!(Eval, eval);

    #[derive(Bpaf, Clone, Debug)]
    pub struct Flake {
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
        /// build and publish project to flox channel
        #[bpaf(command, hide)]
        Publish2(#[bpaf(external(WithPassthru::parse))] WithPassthru<PublishV2>),
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
        /// containerize an environment
        #[bpaf(command)]
        Containerize(#[bpaf(external(WithPassthru::parse))] WithPassthru<Containerize>),
        /// run `nix flake` commands
        #[bpaf(command)]
        Flake(#[bpaf(external(WithPassthru::parse))] WithPassthru<Flake>),
    }
}

impl PackageCommands {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            PackageCommands::Develop(_) => subcommand_metric!("develop"),
            PackageCommands::Init(_) => subcommand_metric!("init"),
            PackageCommands::Build(_) => subcommand_metric!("build"),
            PackageCommands::PrintDevEnv(_) => subcommand_metric!("print-dev-env"),
            PackageCommands::Publish(_) => subcommand_metric!("publish"),
            PackageCommands::Publish2(_) => subcommand_metric!("publish_v2"),
            PackageCommands::Run(_) => subcommand_metric!("run"),
            PackageCommands::Shell(_) => subcommand_metric!("shell"),
            PackageCommands::Eval(_) => subcommand_metric!("eval"),
            PackageCommands::Bundle(_) => subcommand_metric!("bundle"),
            PackageCommands::Containerize(_) => subcommand_metric!("containerize"),
            PackageCommands::Flake(_) => subcommand_metric!("flake"),
        }

        match self {
            _ if Feature::Nix.is_forwarded()? => flox_forward(&flox).await?,

            // Unification implementation of Develop is not yet implemented in rust
            PackageCommands::Develop(_) if Feature::Develop.is_forwarded()? => {
                flox_forward(&flox).await?
            },

            // Unification implementation of print-dev-env is not yet implemented in rust
            PackageCommands::PrintDevEnv(_) if Feature::Develop.is_forwarded()? => {
                flox_forward(&flox).await?
            },

            // `flox publish` is not yet implemented in rust
            PackageCommands::Publish(_) if Feature::Publish.is_forwarded()? => {
                flox_forward(&flox).await?
            },

            PackageCommands::Publish2(args) => {
                let FlakeAttribute {
                    flakeref,
                    attr_path,
                } = args
                    .inner
                    .installable_arg
                    .unwrap_or_default()
                    .resolve_flake_attribute(&flox)
                    .await?;

                let publish_ref = PublishFlakeRef::from_flake_ref(flakeref, &flox, false).await?;
                let publish = Publish::new(&flox, publish_ref, attr_path, config.flox.stability);
                println!(
                    "{}",
                    serde_json::to_string_pretty(publish.analyze().await?.analysis())?
                );
            },

            PackageCommands::Init(command) => {
                let cwd = std::env::current_dir()?;
                let basename = cwd
                    .file_name()
                    .and_then(|x| x.to_str())
                    .unwrap_or("NAME")
                    .to_owned();

                let git_repo = ensure_project_repo(&flox, cwd).await?;
                let project = ensure_project(git_repo, &command).await?;

                // Check if template exists before asking for project's name
                let template = command
                    .inner
                    .template
                    .unwrap_or_default()
                    .resolve_flake_attribute(&flox)
                    .await?
                    .into();

                let name = match command.inner.name {
                    Some(n) => n,
                    None => {
                        // Comment this out since we're using mkShell instead of
                        // root-level flox.nix
                        // TODO: find a better way to not hardcode this
                        // if template.to_string() == "flake:flox#.templates.project" {
                        //     "default".to_string()
                        // } else {
                        let dialog = Dialog {
                            message: "Enter package name",
                            help_message: None,
                            typed: Text {
                                default: Some(&basename),
                            },
                        };

                        dialog.prompt().await.context("Failed to prompt for name")?
                        // }
                    },
                };

                let name = name.trim();

                if !name.is_empty() {
                    project
                        .init_flox_package::<NixCommandLine>(command.nix_args, template, name)
                        .await?;
                }

                info!("Run 'flox develop' to enter the project environment.")
            },
            PackageCommands::Build(command) => {
                let installable_arg = command
                    .inner
                    .installable_arg
                    .unwrap_or_default()
                    .resolve_flake_attribute(&flox)
                    .await?;

                flox.package(installable_arg, config.flox.stability, command.nix_args)
                    .build::<NixCommandLine>()
                    .await?;
            },
            PackageCommands::Develop(command) => {
                let installable_arg = command
                    .inner
                    .installable_arg
                    .unwrap_or_default()
                    .resolve_flake_attribute(&flox)
                    .await?;

                flox.package(installable_arg, config.flox.stability, command.nix_args)
                    .develop::<NixCommandLine>()
                    .await?
            },
            PackageCommands::Run(command) => {
                let installable_arg = command
                    .inner
                    .installable_arg
                    .unwrap_or_default()
                    .resolve_flake_attribute(&flox)
                    .await?;

                flox.package(installable_arg, config.flox.stability, command.nix_args)
                    .run::<NixCommandLine>()
                    .await?
            },
            PackageCommands::Shell(command) => {
                let installable_arg = command
                    .inner
                    .installable_arg
                    .unwrap_or_default()
                    .resolve_flake_attribute(&flox)
                    .await?;

                flox.package(installable_arg, config.flox.stability, command.nix_args)
                    .shell::<NixCommandLine>()
                    .await?
            },
            PackageCommands::Eval(command) => {
                let nix = flox.nix::<NixCommandLine>(command.nix_args);
                let command = EvalComm {
                    flake: FlakeArgs {
                        override_inputs: [config.flox.stability.as_override()].into(),
                        ..FlakeArgs::default()
                    },
                    ..Default::default()
                };

                command.run(&nix, &NixArgs::default()).await?
            },
            PackageCommands::Bundle(command) => {
                let installable_arg = ResolveInstallable::<GitCommandProvider>::installable(
                    &command.inner.installable_arg,
                    &flox,
                )
                .await?;

                let bundler = command
                    .inner
                    .bundler_arg
                    .unwrap_or_default()
                    .resolve_flake_attribute(&flox)
                    .await?;

                flox.package(installable_arg, config.flox.stability, command.nix_args)
                    .bundle::<NixCommandLine>(bundler.into())
                    .await?
            },
            PackageCommands::Containerize(command) => {
                let mut installable = env_ref_to_flake_attribute::<GitCommandProvider>(
                    &flox,
                    "containerize",
                    &command.inner.environment_name.unwrap_or_default(),
                )
                .await?;

                installable
                    .attr_path
                    .extend(["passthru", "streamLayeredImage"].map(|attr| attr.parse().unwrap()));

                if std::io::stdout().is_tty() {
                    bail!(
                        indoc! {"
                        'flox containerize' pipes a container image to stdout, but stdout is
                        attached to the terminal. Instead, run this command as:

                            $ {command} | docker load
                    "},
                        command = env::args()
                            .map(|arg| shell_escape::escape(arg.into()))
                            .join(" ")
                    );
                }

                let nix = flox.nix::<NixCommandLine>(command.nix_args);

                let nix_args = NixArgs::default();

                info!("Building container...");

                let command = Build {
                    installables: [installable.into()].into(),
                    eval: EvaluationArgs {
                        impure: true.into(),
                    },
                    ..Default::default()
                };

                let mut out: BuildOut = command.run_typed(&nix, &nix_args).await?;

                info!("Done.");

                let script = out
                    .pop()
                    .context("Container script not built")?
                    .outputs
                    .remove("out")
                    .context("Container script output not found")?;

                debug!("Got container script: {:?}", script);

                tokio::process::Command::new(script)
                    .spawn()
                    .context("Failed to start container script")?
                    .wait()
                    .await
                    .context("Container script failed to run")?;
            },
            PackageCommands::Flake(command) => {
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
                        override_inputs: [config.flox.stability.as_override()].into(),
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

async fn ensure_project_repo(
    flox: &Flox,
    cwd: PathBuf,
) -> Result<root::Root<Closed<GitCommandProvider>>, anyhow::Error> {
    match flox
        .resource(cwd)
        .guard::<GitCommandProvider>()
        .await?
        .open()
    {
        Ok(p) => {
            info!(
                "Found git repo{}",
                p.workdir()
                    .map(|p| format!(": {}", p.display()))
                    .unwrap_or_else(|| "".to_owned())
            );
            Ok(p)
        },
        Err(_) => bail!(indoc! {"
            You must be inside of a Git repository to initialize a project

            To provide the best possible experience, projects must be under version control.
            Please initialize a project in an existing repo or create one using 'git init'.
        "}),
    }
}

/// Create
async fn ensure_project<'flox>(
    git_repo: Root<'flox, Closed<GitCommandProvider>>,
    command: &WithPassthru<interface::Init>,
) -> Result<Project<'flox, GitCommandProvider, ReadOnly<GitCommandProvider>>> {
    match git_repo.guard().await?.open() {
        Ok(x) => Ok(x),
        Err(g) => Ok(g
            .init_project::<NixCommandLine>(command.nix_args.clone())
            .await?),
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct PackageArgs {
    #[bpaf(long, argument("STABILITY"))]
    pub stability: Option<Stability>,
}

// impl PackageArgs {
//     /// Resolve stability from flag or config (which reads environment variables).
//     /// If the stability is set by a flag, modify STABILITY env variable to match
//     /// the set stability.
//     /// Flox invocations in a child process will inherit hence inherit the stability.
//     pub(crate) fn stability(&self, config: &Config) -> Stability {
//         if let Some(ref stability) = self.stability {
//             env::set_var("FLOX_STABILITY", stability.to_string());
//             stability.clone()
//         } else {
//             config.flox.stability.clone()
//         }
//     }
// }

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
    /// and implements the [Parseable] trait for it
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
