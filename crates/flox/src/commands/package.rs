use std::env;
use std::fmt::Debug;

use anyhow::{bail, Context, Result};
use bpaf::{construct, Bpaf, Parser};
use crossterm::tty::IsTty;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::nix::arguments::eval::EvaluationArgs;
use flox_rust_sdk::nix::arguments::flake::FlakeArgs;
use flox_rust_sdk::nix::arguments::NixArgs;
use flox_rust_sdk::nix::command::{Build as BuildComm, BuildOut};
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::nix::RunTyped;
use flox_rust_sdk::prelude::FlakeAttribute;
use flox_rust_sdk::providers::git::{GitCommandProvider, GitProvider};
use flox_types::stability::Stability;
use indoc::indoc;
use itertools::Itertools;
use log::{debug, info};

use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::resolve_environment_ref;

async fn env_ref_to_flake_attribute<Git: GitProvider + 'static>(
    flox: &Flox,
    subcommand: &str,
    environment_name: &str,
) -> anyhow::Result<FlakeAttribute> {
    let env_ref =
        resolve_environment_ref::<GitCommandProvider>(flox, subcommand, Some(environment_name))
            .await?;
    Ok(env_ref.get_latest_flake_attribute::<Git>(flox).await?)
}

use async_trait::async_trait;

use self::parseable_macro::parseable;
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
