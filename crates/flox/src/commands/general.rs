use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::nix::command_line::{Group, NixCliCommand, NixCommandLine, ToArgs};
use flox_rust_sdk::nix::Run;
use fslock::LockFile;

use crate::config::{Feature, Impl};
use crate::utils::init::init_telemetry_consent;
use crate::utils::metrics::{
    METRICS_EVENTS_FILE_NAME,
    METRICS_LOCK_FILE_NAME,
    METRICS_UUID_FILE_NAME,
};
use crate::{flox_forward, should_flox_forward, subcommand_metric};

#[derive(Bpaf, Clone)]
pub struct GeneralArgs {}

impl GeneralCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            GeneralCommands::Nix(args) if Feature::Nix.implementation()? == Impl::Rust => {
                subcommand_metric!("nix");

                let nix: NixCommandLine = flox.nix(Default::default());
                RawCommand::new(args.to_owned())
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

            _ if should_flox_forward(Feature::All)? => flox_forward(&flox).await?,
            _ => todo!(),
        }
        Ok(())
    }
}

#[derive(Bpaf, Clone)]
pub enum GeneralCommands {
    /// access to the gh CLI
    #[bpaf(command, hide)]
    Gh(Vec<String>),

    #[bpaf(command)]
    Nix(#[bpaf(any("NIX ARGUMENTS"), complete_shell(complete_nix_shell()))] Vec<String>),

    /// configure user parameters
    #[bpaf(command)]
    Config(#[bpaf(external(config_args))] ConfigArgs),

    /// list all available environments
    #[bpaf(command, long("environments"))]
    Envs,

    /// reset the metrics queue (if any), reset metrics ID, and re-prompt for consent
    #[bpaf(command("reset-metrics"))]
    ResetMetrics,
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

fn complete_nix_shell() -> bpaf::ShellComp {
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
        zsh: Box::leak(format!("source {}", env!("NIX_ZSH_COMPLETION_SCRIPT")).into_boxed_str()),
        bash: Box::leak(
            format!(
                "source {}; _nix_bash_completion",
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
