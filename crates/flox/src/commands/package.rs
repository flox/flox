use std::fmt::Debug;

use bpaf::Bpaf;
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
