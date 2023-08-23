use std::path::Path;
use std::str::FromStr;
use std::{env, io};

use anyhow::{Context, Result};
use bpaf::{Bpaf, Parser};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::nix::command_line::{Group, NixCliCommand, NixCommandLine, ToArgs};
use flox_rust_sdk::nix::Run;
use flox_rust_sdk::prelude::Channel;
use flox_types::stability::Stability;
use fslock::LockFile;
use indoc::indoc;
use log::info;
use serde::Serialize;
use tokio::fs;
use toml_edit::Key;

use crate::commands::not_help;
use crate::config::features::Feature;
use crate::config::{Config, ReadWriteError, FLOX_CONFIG_FILE};
use crate::utils::metrics::{
    METRICS_EVENTS_FILE_NAME,
    METRICS_LOCK_FILE_NAME,
    METRICS_UUID_FILE_NAME,
};
use crate::{flox_forward, subcommand_metric};

#[derive(Bpaf, Clone)]
pub struct GeneralArgs {}

impl GeneralCommands {
    pub async fn handle(self, mut config: Config, mut flox: Flox) -> Result<()> {
        match self {
            GeneralCommands::Gh(_) => subcommand_metric!("gh"),
            GeneralCommands::Config(_) => subcommand_metric!("config"),
            GeneralCommands::ResetMetrics(_) => subcommand_metric!("reset-metrics"),
            GeneralCommands::Nix(_) => subcommand_metric!("nix"),
        }

        match self {
            GeneralCommands::Nix(_) if Feature::Nix.is_forwarded()? => flox_forward(&flox).await?,

            // To be moved to packages - figure out completions again
            GeneralCommands::Nix(wrapped) => wrapped.handle(config, flox).await?,

            GeneralCommands::ResetMetrics(args) => args.handle(config, flox).await?,

            GeneralCommands::Config(config_args) => config_args.handle(config, flox).await?,

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
    Gh(#[bpaf(any("gh Arguments", Some))] Vec<String>),

    /// configure user parameters
    #[bpaf(command)]
    Config(#[bpaf(external(config_args))] ConfigArgs),

    /// reset the metrics queue (if any), reset metrics ID, and re-prompt for consent
    #[bpaf(command("reset-metrics"))]
    ResetMetrics(#[bpaf(external(reset_metrics))] ResetMetrics),

    /// access to the nix CLI
    Nix(#[bpaf(external(parse_nix_passthru))] WrappedNix),
}

#[derive(Bpaf, Clone)]
pub struct ResetMetrics {}
impl ResetMetrics {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        let mut metrics_lock = LockFile::open(&flox.cache_dir.join(METRICS_LOCK_FILE_NAME))?;
        tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

        if let Err(err) =
            tokio::fs::remove_file(flox.cache_dir.join(METRICS_EVENTS_FILE_NAME)).await
        {
            match err.kind() {
                std::io::ErrorKind::NotFound => {},
                _ => Err(err)?,
            }
        }

        if let Err(err) = tokio::fs::remove_file(flox.data_dir.join(METRICS_UUID_FILE_NAME)).await {
            match err.kind() {
                std::io::ErrorKind::NotFound => {},
                _ => Err(err)?,
            }
        }

        let notice = indoc! {"
                    Sucessfully reset telemetry ID for this machine!

                    A new ID will be assigned next time you use flox.

                    The collection of metrics can be disabled in the following ways:

                      environment: FLOX_DISABLE_METRICS=true
                        user-wide: flox config --set-bool disable_metrics true
                      system-wide: update /etc/flox.toml as described in flox(1)
                "};

        info!("{notice}");
        Ok(())
    }
}

#[derive(Bpaf, Clone)]
#[bpaf(fallback(ConfigArgs::List))]
pub enum ConfigArgs {
    /// list the current values of all configurable paramers
    #[bpaf(short, long)]
    List,
    /// reset all configurable parameters to their default values without further confirmation.
    #[bpaf(short, long)]
    Reset,
    Set(#[bpaf(external(config_set))] ConfigSet),
    SetNumber(#[bpaf(external(config_set_number))] ConfigSetNumber),
    SetBool(#[bpaf(external(config_set_bool))] ConfigSetBool),
    Delete(#[bpaf(external(config_delete))] ConfigDelete),
}

impl ConfigArgs {
    /// handle config flags like commands
    async fn handle(&self, config: Config, flox: Flox) -> Result<()> {
        /// wrapper around [Config::write_to]
        async fn update_config<V: Serialize>(
            config_dir: &Path,
            temp_dir: &Path,
            key: impl AsRef<str>,
            value: Option<V>,
        ) -> Result<()> {
            let query = Key::parse(key.as_ref()).context("Could not parse key")?;

            let config_file_path = config_dir.join(FLOX_CONFIG_FILE);

            match Config::write_to_in(config_file_path, temp_dir, &query, value) {
                err @ Err(ReadWriteError::ReadConfig(_)) => err.context("Could not read current config file.\nPlease verify the format or reset using `flox config --reset`")?,
                err @ Err(_) => err?,
                Ok(()) => ()
            }
            Ok(())
        }

        match self {
            ConfigArgs::List => println!("{}", config.get(&[])?),
            ConfigArgs::Reset => {
                match fs::remove_file(&flox.config_dir.join(FLOX_CONFIG_FILE)).await {
                    Err(err) if err.kind() != io::ErrorKind::NotFound => {
                        Err(err).context("Could not reset config file")?
                    },
                    _ => (),
                }
            },
            ConfigArgs::Set(ConfigSet { key, value, .. }) => {
                update_config(&flox.config_dir, &flox.temp_dir, key, Some(value)).await?
            },
            ConfigArgs::SetNumber(ConfigSetNumber { key, value, .. }) => {
                update_config(
                    &flox.config_dir,
                    &flox.temp_dir,
                    key,
                    Some(
                        value
                            .parse::<i32>()
                            .context(format!("could not parse '{value}' as number"))?,
                    ),
                )
                .await?
            },
            ConfigArgs::SetBool(ConfigSetBool { key, value, .. }) => {
                update_config(
                    &flox.config_dir,
                    &flox.temp_dir,
                    key,
                    Some(
                        value
                            .parse::<bool>()
                            .context(format!("could not parse '{value}' as bool"))?,
                    ),
                )
                .await?
            },
            ConfigArgs::Delete(ConfigDelete { key, .. }) => {
                update_config::<()>(&flox.config_dir, &flox.temp_dir, key, None).await?
            },
        }
        Ok(())
    }
}

/// Arguments for `flox config --set`
#[derive(Debug, Clone, Bpaf)]
#[bpaf(adjacent)]
#[allow(unused)]
pub struct ConfigSet {
    /// set <key> to <value>
    set: (),
    /// Configuration key
    #[bpaf(positional("key"))]
    key: String,
    /// configuration Value
    #[bpaf(positional("value"))]
    value: String,
}

/// Arguments for `flox config --setNumber`
#[derive(Debug, Clone, Bpaf)]
#[bpaf(adjacent)]
#[allow(unused)]
pub struct ConfigSetNumber {
    /// Set <key> to <number>
    #[bpaf(long("set-number"))]
    set_number: (),
    /// Configuration key
    #[bpaf(positional("key"))]
    key: String,
    /// Configuration Value (i32)
    // we have to parse to int ourselves after reading the argument,
    // as the bpaf error for parse failures here is not descriptive enough
    // (<https://github.com/pacak/bpaf/issues/172>)
    #[bpaf(positional("number"))]
    value: String,
}

/// Arguments for `flox config --setNumber`
#[derive(Debug, Clone, Bpaf)]
#[bpaf(adjacent)]
#[allow(unused)]
pub struct ConfigSetBool {
    /// Set <key> to <bool>
    #[bpaf(long("set-bool"))]
    set_bool: (),
    /// Configuration key
    #[bpaf(positional("key"))]
    key: String,
    /// Configuration Value (bool)
    #[bpaf(positional("bool"))]
    // #[bpaf(external(parse_bool))]
    // we have to parse to int ourselves after reading the argument,
    // as the bpaf error for parse failures here is not descriptive enough
    // (<https://github.com/pacak/bpaf/issues/172>)
    value: String,
}

/// bug in bpaf (<https://github.com/pacak/bpaf/issues/171>)
// fn parse_bool() -> impl Parser<String> {
//     bpaf::positional::<String>("bool")
// }

/// Arguments for `flox config --delete`
#[derive(Debug, Clone, Bpaf)]
#[bpaf(adjacent)]
#[allow(unused)]
/// delete <key> from config
pub struct ConfigDelete {
    /// Configuration key
    #[bpaf(long("delete"), argument("key"))]
    key: String,
}

/// Access to the nix CLI
#[derive(Clone, Debug)]
pub struct WrappedNix {
    stability: Option<Stability>,
    nix_args: Vec<String>,
}

impl WrappedNix {
    pub async fn handle(self, mut config: Config, mut flox: Flox) -> Result<()> {
        // mutable state hurray :/
        config.flox.stability = {
            if let Some(ref stability) = self.stability {
                env::set_var("FLOX_STABILITY", stability.to_string());
                stability.clone()
            } else {
                config.flox.stability
            }
        };

        if config.flox.stability != Default::default() {
            flox.channels.register_channel(
                "nixpkgs",
                Channel::from_str(&format!("github:flox/nixpkgs/{}", config.flox.stability))?,
            );
        }

        let nix: NixCommandLine = flox.nix(Default::default());

        RawCommand::new(self.nix_args.to_owned())
            .run(&nix, &Default::default())
            .await?;
        Ok(())
    }
}

pub fn parse_nix_passthru() -> impl Parser<WrappedNix> {
    fn nix_sub_command<const OFFSET: u8>() -> impl Parser<Vec<String>> {
        let free = bpaf::any("NIX ARGUMENTS", not_help)
            .complete_shell(complete_nix_shell(OFFSET))
            .many();

        let strict = bpaf::positional("NIX ARGUMENTS AND OPTIONS")
            .strict()
            .many();

        bpaf::construct!(free, strict).map(|(free, strict)| [free, strict].concat())
    }

    let with_stability = {
        let stability = bpaf::long("stability").argument("STABILITY").map(Some);
        let nix_args = nix_sub_command::<2>();
        bpaf::construct!(WrappedNix {
            stability,
            nix_args
        })
        .adjacent()
    };

    let without_stability = {
        let stability = bpaf::pure(Default::default());
        let nix_args = nix_sub_command::<0>().hide();
        bpaf::construct!(WrappedNix {
            nix_args,
            stability
        })
        .hide()
    };

    bpaf::construct!([without_stability, with_stability])
        .to_options()
        .command("nix")
        .help("Access to the nix CLI")
}

fn complete_nix_shell(offset: u8) -> bpaf::ShellComp {
    // Box::leak will effectively turn the String
    // (that is produced by `replace`) insto a `&'static str`,
    // at the cost of giving up memory management over that string.
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
/// Doesn't permit the application of any default arguments set by flox,
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
