use std::env;
use std::str::FromStr;

use anyhow::Result;
use bpaf::{Bpaf, Parser};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::nix::command_line::{Group, NixCliCommand, NixCommandLine, ToArgs};
use flox_rust_sdk::nix::Run;
use flox_rust_sdk::prelude::{Channel, Stability};
use fslock::LockFile;

use crate::config::features::Feature;
use crate::config::Config;
use crate::utils::init::init_telemetry_consent;
use crate::utils::metrics::{
    METRICS_EVENTS_FILE_NAME,
    METRICS_LOCK_FILE_NAME,
    METRICS_UUID_FILE_NAME,
};
use crate::{flox_forward, subcommand_metric};

#[derive(Bpaf, Clone)]
pub struct GeneralArgs {}

impl GeneralCommands {
    pub async fn handle(&self, mut config: Config, mut flox: Flox) -> Result<()> {
        match self {
            GeneralCommands::Nix(_) if Feature::Nix.is_forwarded()? => flox_forward(&flox).await?,

            // To be moved to packages - figure out completions again
            GeneralCommands::Nix(wrapped) => {
                subcommand_metric!("nix");

                // mutable state hurray :/
                config.flox.stability = {
                    if let Some(ref stability) = wrapped.stability {
                        env::set_var("FLOX_STABILITY", stability.to_string());
                        stability.clone()
                    } else {
                        config.flox.stability
                    }
                };

                flox.channels.register_channel(
                    "nixpkgs",
                    Channel::from_str(&format!("github:flox/nixpkgs/{}", config.flox.stability))?,
                );

                let nix: NixCommandLine = flox.nix(Default::default());

                RawCommand::new(wrapped.nix.to_owned())
                    .run(&nix, &Default::default())
                    .await?;
            },

            GeneralCommands::ResetMetrics => {
                let mut metrics_lock =
                    LockFile::open(&flox.cache_dir.join(METRICS_LOCK_FILE_NAME))?;
                tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

                if let Err(err) =
                    tokio::fs::remove_file(flox.cache_dir.join(METRICS_EVENTS_FILE_NAME)).await
                {
                    match err.kind() {
                        std::io::ErrorKind::NotFound => {},
                        _ => Err(err)?,
                    }
                }

                if let Err(err) =
                    tokio::fs::remove_file(flox.data_dir.join(METRICS_UUID_FILE_NAME)).await
                {
                    match err.kind() {
                        std::io::ErrorKind::NotFound => {},
                        _ => Err(err)?,
                    }
                }

                init_telemetry_consent(&flox.data_dir, &flox.cache_dir).await?;
            },
            _ if Feature::All.is_forwarded()? => flox_forward(&flox).await?,
            _ => todo!(),
        }
        Ok(())
    }
}

/// General Commands
#[derive(Bpaf, Clone)]
pub enum GeneralCommands {
    /// access to the gh CLI
    #[bpaf(command, hide)]
    Gh(Vec<String>),

    /// configure user parameters
    #[bpaf(command)]
    Config(#[bpaf(external(config_args))] ConfigArgs),

    /// list all available environments
    /// Aliases:
    ///   environments, envs
    #[bpaf(command, long("environments"))]
    Envs,

    /// reset the metrics queue (if any), reset metrics ID, and re-prompt for consent
    #[bpaf(command("reset-metrics"))]
    ResetMetrics,

    /// access to the nix CLI
    Nix(#[bpaf(external(parse_nix_passthru))] WrappedNix),
}

#[derive(Bpaf, Clone)]
pub enum ConfigArgs {
    /// list the current values of all configurable paramers
    #[bpaf(short, long)]
    List,
    /// prompt the user to confirm or update configurable parameters.
    #[bpaf(short, long)]
    Remove,
    /// reset all configurable parameters to their default values without further confirmation.
    #[bpaf(short, long)]
    Confirm,
}

#[derive(Clone, Debug)]
pub struct WrappedNix {
    stability: Option<Stability>,
    nix: Vec<String>,
}
fn parse_nix_passthru() -> impl Parser<WrappedNix> {
    fn nix_sub_command<const OFFSET: u8>() -> impl Parser<Vec<String>> {
        bpaf::command(
            "nix",
            bpaf::any("NIX ARGUMENTS")
                .guard(|item| item != "--stability", "Stability not expected")
                .complete_shell(complete_nix_shell(OFFSET))
                .many()
                .to_options(),
        )
        .help("access to the nix CLI")
    }
    let with_stability = {
        let stability = bpaf::long("stability").argument("STABILITY").map(Some);
        let nix = nix_sub_command::<2>();
        bpaf::construct!(WrappedNix { stability, nix }).adjacent()
    };

    let without_stability = {
        let stability = bpaf::pure(Default::default());
        let nix = nix_sub_command::<0>().hide();
        bpaf::construct!(WrappedNix { nix, stability }).hide()
    };

    bpaf::construct!([without_stability, with_stability]).hide_usage()
}

fn complete_nix_shell(offset: u8) -> bpaf::ShellComp {
    // Box::leak will effectively turn the String
    // (that is produced by `replace`) insto a `&'static str`,
    // at the cost of giving up memeory management over that string.
    //
    // Note:
    // We could use a `OnceCell` to ensure this leak happens only once.
    // However, this should not be necessary after all,
    // since the completion runs in its own process.
    // Any memory it leaks will be cleared by the system allocator.
    bpaf::ShellComp::Raw {
        zsh: Box::leak(
            format!(
                "OFFSET={}; echo 'was' > /dev/stderr; source {}",
                offset,
                env!("NIX_ZSH_COMPLETION_SCRIPT")
            )
            .into_boxed_str(),
        ),
        bash: Box::leak(
            format!(
                "OFFSET={}; source {}; _nix_bash_completion",
                offset,
                env!("NIX_BASH_COMPLETION_SCRIPT")
            )
            .into_boxed_str(),
        ),
        fish: "",
        elvish: "",
    }
}

/// A raw nix command.
///
/// Will run `nix <default args> <self.args>...`
///
/// Doesn't permit the appplication of any default arguments set by flox,
/// except nix configuration args and common nix command args.
///
/// See: [`nix --help`](https://nixos.org/manual/nix/unstable/command-ref/new-cli/nix.html)
#[derive(Debug, Clone)]
pub struct RawCommand {
    args: Vec<String>,
}

impl RawCommand {
    fn new(args: Vec<String>) -> Self {
        RawCommand { args }
    }
}
impl ToArgs for RawCommand {
    fn to_args(&self) -> Vec<String> {
        self.args.to_owned()
    }
}

impl NixCliCommand for RawCommand {
    type Own = Self;

    const OWN_ARGS: Group<Self, Self::Own> = Some(|s| s.to_owned());
    const SUBCOMMAND: &'static [&'static str] = &[];
}
