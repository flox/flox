use std::env;
use std::fmt::Debug;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use bpaf::{construct, Bpaf, Parser};
use crossterm::tty::IsTty;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::project::Project;
use flox_rust_sdk::models::publish::{Publish as PublishComm, PublishFlakeRef};
use flox_rust_sdk::models::root::transaction::ReadOnly;
use flox_rust_sdk::models::root::{self, Closed, Root};
use flox_rust_sdk::nix::arguments::eval::EvaluationArgs;
use flox_rust_sdk::nix::arguments::flake::FlakeArgs;
use flox_rust_sdk::nix::arguments::NixArgs;
use flox_rust_sdk::nix::command::{Build as BuildComm, BuildOut, Eval as EvalComm};
use flox_rust_sdk::nix::command_line::{Group, NixCliCommand, NixCommandLine, ToArgs};
use flox_rust_sdk::nix::{Run as RunC, RunTyped};
use flox_rust_sdk::prelude::FlakeAttribute;
use flox_rust_sdk::providers::git::{GitCommandProvider, GitProvider};
use flox_types::stability::Stability;
use indoc::indoc;
use itertools::Itertools;
use log::{debug, info};

use crate::config::Config;
use crate::utils::dialog::{Dialog, Text};
use crate::utils::resolve_environment_ref;
use crate::{flox_forward, subcommand_metric};

async fn env_ref_to_flake_attribute<Git: GitProvider + 'static>(
    flox: &Flox,
    subcommand: &str,
    environment_name: &str,
) -> anyhow::Result<FlakeAttribute> {
    let env_ref = resolve_environment_ref(flox, subcommand, Some(environment_name)).await?;
    Ok(env_ref.get_latest_flake_attribute::<Git>(flox).await?)
}

use async_trait::async_trait;
use flox_types::catalog::cache::SubstituterUrl;

use self::parseable_macro::parseable;
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
    fn parse() -> Box<dyn bpaf::Parser<Self>> {
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
            PosOrEnv::Env(n) => env_ref_to_flake_attribute::<Git>(flox, T::SUBCOMMAND, n).await?,
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
pub struct InitPackage {
    // [sic]
    // template does NOT support package args
    // - e.g. `stability`
    #[bpaf(external(template_arg))]
    pub(crate) template: Option<InstallableArgument<Parsed, TemplateInstallable>>,
    #[bpaf(long("name"), short('n'), argument("name"))]
    pub(crate) name: Option<String>,
}
pub(crate) fn template_arg() -> impl Parser<Option<InstallableArgument<Parsed, TemplateInstallable>>>
{
    InstallableArgument::parse_with(bpaf::long("template").short('t').argument("template"))
        .optional()
}
parseable!(InitPackage, init_package);
impl WithPassthru<InitPackage> {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("init-package");
        let cwd = std::env::current_dir()?;
        let basename = cwd
            .file_name()
            .and_then(|x| x.to_str())
            .unwrap_or("NAME")
            .to_owned();

        let git_repo = ensure_project_repo(&flox, cwd).await?;
        let project = ensure_project(git_repo, &self).await?;

        // Check if template exists before asking for project's name
        let template = self
            .inner
            .template
            .unwrap_or_default()
            .resolve_flake_attribute(&flox)
            .await?
            .into();

        let name = match self.inner.name {
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
                .init_flox_package::<NixCommandLine>(self.nix_args, template, name)
                .await?;
        }

        info!("Run 'flox develop' to enter the project environment.");
        Ok(())
    }
}

#[derive(Debug, Clone, Bpaf)]
pub struct Build {
    #[bpaf(short('A'), hide)]
    pub(crate) _attr_flag: bool,

    #[bpaf(external(InstallableArgument::positional), optional, catch)]
    pub(crate) installable_arg: Option<InstallableArgument<Parsed, BuildInstallable>>,
}
parseable!(Build, build);
impl WithPassthru<Build> {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("build");
        let installable_arg = self
            .inner
            .installable_arg
            .unwrap_or_default()
            .resolve_flake_attribute(&flox)
            .await?;

        let stability: Option<Stability> = config.override_stability(self.stability);

        flox.package(installable_arg, stability, self.nix_args)
            .build::<NixCommandLine>()
            .await?;
        Ok(())
    }
}

/// print shell code that can be sourced by bash to reproduce the development environment
#[derive(Bpaf, Clone, Debug)]
pub struct PrintDevEnv {
    #[bpaf(short('A'), hide)]
    pub _attr_flag: bool,

    /// Shell or package to develop on
    #[bpaf(external(InstallableArgument::positional), optional, catch)]
    pub(crate) _installable_arg: Option<InstallableArgument<Parsed, DevelopInstallable>>,
}
parseable!(PrintDevEnv, print_dev_env);
impl WithPassthru<PrintDevEnv> {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("print-dev-env");
        config.override_stability(self.stability);

        flox_forward(&flox).await
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct Publish {
    /// Signing key file to sign the binary with
    ///
    /// When omitted, reads from the config.
    /// See flox-config(1) for more details.
    #[bpaf(long, short('k'))]
    pub signing_key: Option<PathBuf>,

    /// Url of a binary cache to push binaries _to_
    ///
    /// When omitted, reads from the config.
    /// See flox-config(1) for more details.
    #[bpaf(long, short('c'))]
    pub cache_url: Option<SubstituterUrl>,

    /// URL of a substituter to pull binaries _from_
    ///
    /// When ommitted, falls back to the config or uses the value for cache-url.
    /// See flox-config(1) for more details.
    #[bpaf(long, short('s'))]
    pub public_cache_url: Option<SubstituterUrl>,

    /// Print snapshot JSON to stdout instead of uploading it to the catalog
    #[bpaf(long, hide)]
    pub json: bool,

    /// Prefer https access to repositories published with a `github:` reference
    ///
    /// `ssh` is used by default.
    #[bpaf(long)]
    pub prefer_https: bool,

    /// Stability to publish
    #[bpaf(long, short)]
    pub stability: Option<Stability>,

    /// Package to publish
    #[bpaf(external(InstallableArgument::positional), optional, catch)]
    pub installable_arg: Option<InstallableArgument<Parsed, PublishInstallable>>,
}
parseable!(Publish, publish);
impl Publish {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("publish");
        let installable = self
            .installable_arg
            .unwrap_or_default()
            .resolve_flake_attribute(&flox)
            .await?;

        let original_flakeref = &installable.flakeref;
        let publish_flakeref =
            PublishFlakeRef::from_flake_ref(installable.flakeref.clone(), &flox, self.prefer_https)
                .await?;

        if &publish_flakeref != original_flakeref {
            info!("Resolved {} to {}", original_flakeref, publish_flakeref);
        }

        // validate arguments

        let stability = config.override_stability(self.stability);

        let sign_key = self
            .signing_key
            .or(config.flox.signing_key)
            .ok_or_else(|| {
                anyhow!(indoc! {"
                            Signing key is required!
                            Provide using `--sign-key` or the `sign_key` config key
                        "})
            })?;

        let cache_url = self.cache_url.or(config.flox.cache_url).ok_or_else(|| {
            anyhow!(indoc! {"
                            Cache url is required!
                            Provide using `--cache-url` or the `cache_url` config key
                        "})
        })?;

        let substituter_url = self
            .public_cache_url
            .or(config.flox.public_cache_url)
            .unwrap_or(cache_url.clone());

        // run publish steps

        let publish = PublishComm::new(
            &flox,
            publish_flakeref.clone(),
            installable.attr_path.clone(),
            stability,
        );

        // retrieve eval metadata
        info!("Getting metadata for {installable}...");
        let mut publish = publish.analyze().await?;

        // build binary
        info!("Building {installable}...");
        publish.build().await?;
        info!("done!");

        // sign binary

        info!("Signing binary...");
        publish
            .sign_binary(&sign_key)
            .await
            .with_context(|| format!("Could not sign binary with sign-key {sign_key:?}"))?;
        info!("done!");

        // cache binary
        info!("Uploading binary to {cache_url}...");
        publish
            .upload_binary(Some(cache_url))
            .await
            .context("Failed uploading binary")?;
        info!("done!");

        info!("Checking binary can be downloaded from {substituter_url}...");
        publish
            .check_substituter(substituter_url)
            .await
            .context("Binary cannot be downloaded")?;
        info!("done!");

        if self.json {
            let analysis = publish.analysis();

            println!("{}", serde_json::to_string(analysis)?);
        } else {
            info!("Uploading snapshot to {}...", publish_flakeref.clone_url());
            publish.push_snapshot().await.context("Failed to upload")?;
            info!("done!");
            info!("Publish complete");
        }
        Ok(())
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct Shell {
    #[bpaf(short('A'), hide)]
    pub _attr_flag: bool,

    /// Package to provide in a shell
    #[bpaf(external(InstallableArgument::positional), optional, catch)]
    pub(crate) installable_arg: Option<InstallableArgument<Parsed, ShellInstallable>>,
}
parseable!(Shell, shell);
impl WithPassthru<Shell> {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("shell");
        let installable_arg = self
            .inner
            .installable_arg
            .unwrap_or_default()
            .resolve_flake_attribute(&flox)
            .await?;

        let stability: Option<Stability> = config.override_stability(self.stability);

        flox.package(installable_arg, stability, self.nix_args)
            .shell::<NixCommandLine>()
            .await?;
        Ok(())
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct Bundle {
    #[bpaf(short('A'), hide)]
    pub _attr_flag: bool,

    /// Bundler to use
    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(external)]
    pub(crate) bundler_arg: Option<InstallableArgument<Parsed, BundlerInstallable>>,

    /// Package or environment to bundle
    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(external(PosOrEnv::parse), optional, catch)]
    pub(crate) installable_arg: Option<PosOrEnv<BundleInstallable>>,
}
parseable!(Bundle, bundle);
pub(crate) fn bundler_arg() -> impl Parser<Option<InstallableArgument<Parsed, BundlerInstallable>>>
{
    InstallableArgument::parse_with(bpaf::long("bundler").short('b').argument("bundler")).optional()
}
impl WithPassthru<Bundle> {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("bundle");
        let installable_arg = ResolveInstallable::<GitCommandProvider>::installable(
            &self.inner.installable_arg,
            &flox,
        )
        .await?;

        let bundler = self
            .inner
            .bundler_arg
            .unwrap_or_default()
            .resolve_flake_attribute(&flox)
            .await?;

        let stability: Option<Stability> = config.override_stability(self.stability);

        flox.package(installable_arg, stability, self.nix_args)
            .bundle::<NixCommandLine>(bundler.into())
            .await?;
        Ok(())
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct Containerize {
    #[bpaf(short('A'), hide)]
    pub _attr_flag: bool,

    /// Environment to containerize
    #[bpaf(long("environment"), short('e'), argument("ENV"))]
    pub(crate) environment_name: Option<String>,
}
parseable!(Containerize, containerize);
impl WithPassthru<Containerize> {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("containerize");
        let mut installable = env_ref_to_flake_attribute::<GitCommandProvider>(
            &flox,
            "containerize",
            &self.inner.environment_name.unwrap_or_default(),
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

        let nix = flox.nix::<NixCommandLine>(self.nix_args);

        let nix_args = NixArgs::default();

        info!("Building container...");

        let stability: Option<Stability> = config.override_stability(self.stability);
        let override_input = stability.as_ref().map(Stability::as_override);

        let command = BuildComm {
            flake: FlakeArgs {
                override_inputs: Vec::from_iter(override_input),
                ..Default::default()
            },
            installables: [installable.into()].into(),
            eval: EvaluationArgs {
                impure: true.into(),
                ..Default::default()
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
        Ok(())
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct Run {
    #[bpaf(short('A'), hide)]
    pub(crate) _attr_flag: bool,

    #[bpaf(external(InstallableArgument::positional), optional, catch)]
    pub(crate) installable_arg: Option<InstallableArgument<Parsed, RunInstallable>>,
}
parseable!(Run, run);
impl WithPassthru<Run> {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("run");
        let installable_arg = self
            .inner
            .installable_arg
            .unwrap_or_default()
            .resolve_flake_attribute(&flox)
            .await?;

        let stability: Option<Stability> = config.override_stability(self.stability);

        flox.package(installable_arg, stability, self.nix_args)
            .run::<NixCommandLine>()
            .await?;
        Ok(())
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct Eval {}
parseable!(Eval, eval);
impl WithPassthru<Eval> {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("eval");
        let nix = flox.nix::<NixCommandLine>(self.nix_args);
        let stability = config.override_stability(self.stability);
        let override_input = stability.as_ref().map(Stability::as_override);
        let command = EvalComm {
            flake: FlakeArgs {
                override_inputs: Vec::from_iter(override_input),
                ..FlakeArgs::default()
            },
            ..Default::default()
        };

        command.run(&nix, &NixArgs::default()).await?;
        Ok(())
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct Flake {
    #[bpaf(positional("NIX FLAKE COMMAND"))]
    pub subcommand: String,
}
parseable!(Flake, flake);
impl WithPassthru<Flake> {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("flake");
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

        let stability = config.override_stability(self.stability);
        let override_input = stability.as_ref().map(Stability::as_override);

        // Flake commands should take `--stability`
        // Can't be a default on the `nix` instance, because that will apply it as a flag
        // on `nix flake` rather than `nix flake <subcommand>`.
        // Even though documented as "Common flake-related options",
        // flake args such as `--override-inputs` can not be applied to `nix flake`.
        // Inform [FlakeCommand] about the issued subcommand
        // and inject the flake args through its `ToArgs` implementation.
        FlakeCommand {
            subcommand: self.inner.subcommand.to_owned(),
            default_flake_args: FlakeArgs {
                override_inputs: Vec::from_iter(override_input),
                ..Default::default()
            },
            args: self.nix_args,
        }
        .run(&nix, &Default::default())
        .await?;
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
    command: &WithPassthru<InitPackage>,
) -> Result<Project<'flox, GitCommandProvider, ReadOnly<GitCommandProvider>>> {
    match git_repo.guard().await?.open() {
        Ok(x) => Ok(x),
        Err(g) => Ok(g
            .init_project::<NixCommandLine>(command.nix_args.clone())
            .await?),
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct PackageArgs {}

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
    /// stability to evaluate with
    pub stability: Option<Stability>,

    pub inner: T,
    pub nix_args: Vec<String>,
}

impl<T> WithPassthru<T> {
    fn with_parser(inner: impl Parser<T>) -> impl Parser<Self> {
        let stability = bpaf::long("stability")
            .argument("stability")
            .help("Stability to evaluate with")
            .optional();

        let nix_args = bpaf::positional("args")
            .strict()
            .many()
            .fallback(Default::default())
            .hide();

        let fake_args = bpaf::any("args",
                |m: String| (!["--help", "-h"].contains(&m.as_str())).then_some(m)
            )
            // .strict()
            .many();

        construct!(stability, inner, fake_args, nix_args).map(
            |(stability, inner, mut fake_args, mut nix_args)| {
                nix_args.append(&mut fake_args);
                WithPassthru {
                    stability,
                    inner,
                    nix_args,
                }
            },
        )
    }
}

pub trait Parseable: Sized {
    fn parse() -> Box<dyn bpaf::Parser<Self>>;
}

impl<T: Parseable + Debug + 'static> Parseable for WithPassthru<T> {
    fn parse() -> Box<dyn bpaf::Parser<WithPassthru<T>>> {
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
                fn parse() -> Box<dyn bpaf::Parser<Self>> {
                    let p = $parser();
                    bpaf::construct!(p)
                }
            }
        };
    }
    pub(crate) use parseable;
}
