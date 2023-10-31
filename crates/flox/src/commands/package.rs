use std::fmt::Debug;

use bpaf::{construct, Bpaf, Parser};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::prelude::FlakeAttribute;
use flox_rust_sdk::providers::git::{GitCommandProvider, GitProvider};
use flox_types::stability::Stability;

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
