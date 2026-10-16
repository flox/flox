//! Saved startup selection, delegating all activation to the ordinary handler.

mod selection;
mod startup;

use std::path::{Path, PathBuf};

use anyhow::Result;
use bpaf::Bpaf;
use flox_config::Config;
use flox_rust_sdk::flox::Flox;
use serde_json::json;
use thiserror::Error;

use self::selection::{SelectionStore, StartupSelector};
use super::activate::{Activate, ActivateOptions, ActivateSubcommandOrOptions};
use crate::utils::message;

#[derive(Debug, Error, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case", prefix = "defaults.")]
pub enum DefaultsError {
    #[error("Invalid startup selector: {0}")]
    InvalidSelector(String),
    #[error(
        "Save exactly one startup selector: OWNER/NAME or --dir PATH, not both or multiple targets."
    )]
    ConflictingSelectors,
    #[error(
        "No startup environment is saved. Run 'flox defaults set OWNER/NAME' or 'flox defaults set --dir PATH'."
    )]
    NoSelection,
    #[error(
        "Invalid startup configuration at {path}: {reason}. Repair this file before changing the saved selection."
    )]
    ConfigInvalid { path: PathBuf, reason: String },
    #[error("Could not access startup configuration at {path}: {source:#}")]
    ConfigIo {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
    #[error("Unsupported shell '{0}'. Choose bash, zsh, fish, or tcsh.")]
    UnsupportedShell(String),
}

impl DefaultsError {
    fn io(path: &Path, source: impl Into<anyhow::Error>) -> Self {
        Self::ConfigIo {
            path: path.to_path_buf(),
            source: source.into(),
        }
    }
}

#[derive(Bpaf, Clone)]
pub enum Defaults {
    /// Save one startup environment without activating or building it
    #[bpaf(command)]
    Set {
        /// Explicit path containing .flox; saved as a canonical absolute path
        #[bpaf(long("dir"), short('d'), argument("PATH"), many)]
        directories: Vec<PathBuf>,
        /// Print the mutation result as JSON
        #[bpaf(long)]
        json: bool,
        #[bpaf(positional("OWNER/NAME"), many)]
        references: Vec<String>,
    },
    /// Show saved selection; manual shell integration remains unverified
    #[bpaf(command)]
    List {
        /// Print the selection inventory as JSON
        #[bpaf(long)]
        json: bool,
    },
    /// Disable future startup selection without deleting environments
    #[bpaf(command)]
    Clear {
        /// Print the mutation result as JSON
        #[bpaf(long)]
        json: bool,
    },
    /// Activate the saved environment using ordinary activation behavior
    #[bpaf(command)]
    Activate {
        #[bpaf(external(super::activate::activate_options))]
        options: ActivateOptions,
        #[bpaf(pure(None))]
        selection: Option<StartupSelector>,
    },
    /// Print manual shell setup instructions and a dynamic startup loader
    #[bpaf(command)]
    Setup {
        #[bpaf(long, argument("SHELL"))]
        shell: String,
        /// Print only; no configuration or startup files are written
        #[bpaf(long("print"), req_flag(()))]
        _print: (),
    },
    #[bpaf(command("_activate-if-saved"), hide)]
    Startup {
        #[bpaf(pure(None))]
        selection: Option<StartupSelector>,
    },
}

impl Defaults {
    pub fn subcommand_name(&self) -> &'static str {
        match self {
            Self::Set { .. } => "defaults::set",
            Self::List { .. } => "defaults::list",
            Self::Clear { .. } => "defaults::clear",
            Self::Activate { .. } => "defaults::activate",
            Self::Setup { .. } => "defaults::setup",
            Self::Startup { .. } => "defaults::startup",
        }
    }

    /// Return before auth/telemetry/bootstrap for local operations and absent
    /// startup selection. A present target is read once and carried to dispatch.
    pub fn handle_early(&mut self) -> Option<Result<()>> {
        if let Self::Setup { shell, .. } = self {
            return Some(startup::print_setup(shell).map_err(Into::into));
        }
        if matches!(self, Self::Startup { .. }) && startup::preserve_inherited() {
            return Some(Ok(()));
        }
        let json_output = matches!(
            self,
            Self::Set { json: true, .. } | Self::List { json: true } | Self::Clear { json: true }
        );
        let result = self.prepare();
        match result {
            Ok(true) => None,
            Ok(false) => Some(Ok(())),
            Err(err) => {
                if json_output {
                    let kind: &'static str = (&err).into();
                    println!(
                        "{}",
                        json!({"error": {"code": kind.trim_start_matches("defaults."), "message": err.to_string()}})
                    );
                }
                Some(Err(err.into()))
            },
        }
    }

    fn prepare(&mut self) -> Result<bool, DefaultsError> {
        let config = Config::parse()
            .map_err(|err| DefaultsError::io(Path::new("resolved Flox configuration"), err))?;
        let store = SelectionStore::new(&config.flox.config_dir);
        match self {
            Self::Set {
                references,
                directories,
                json,
            } => {
                let selection = StartupSelector::from_input(references, directories)?;
                let changed = store.save(Some(selection.clone()))?;
                print_inventory(Some(selection), Some(changed), *json);
            },
            Self::List { json } => print_inventory(store.read()?, None, *json),
            Self::Clear { json } => {
                let changed = store.save(None)?;
                print_inventory(None, Some(changed), *json);
            },
            Self::Activate { selection, .. } => {
                *selection = Some(store.read()?.ok_or(DefaultsError::NoSelection)?);
                return Ok(true);
            },
            Self::Startup { selection } => {
                *selection = store.read()?;
                return Ok(selection.is_some());
            },
            Self::Setup { .. } => unreachable!("setup returned before configuration"),
        }
        Ok(false)
    }

    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        let command_name = self.subcommand_name();
        let (selection, options) = match self {
            Self::Activate { selection, options } => (selection, options),
            Self::Startup { selection } => (selection, ActivateOptions {
                print_script: true,
                trust: false,
                start_services: false,
                no_start_services: false,
                add_sbin: false,
                mode: None,
                generation: None,
                command: None,
            }),
            _ => unreachable!("local defaults commands returned before activation dispatch"),
        };
        Activate {
            environment: selection
                .ok_or(DefaultsError::NoSelection)?
                .into_environment(),
            subcommand_or_options: ActivateSubcommandOrOptions::ActivateOptions { options },
        }
        .handle_with_command_name(config, flox, command_name)
        .await
    }
}

fn print_inventory(selection: Option<StartupSelector>, changed: Option<bool>, json_output: bool) {
    if json_output {
        let mut result = json!({"selection": selection, "integration": {"status": "unverified"}});
        if let Some(changed) = changed {
            result["changed"] = json!(changed);
        }
        println!("{result}");
    } else {
        match selection {
            Some(selection) => message::plain(format!(
                "Saved startup selection: {}",
                selection.description()
            )),
            None => message::plain("No startup selection saved."),
        }
        if changed.is_some() {
            message::plain(
                "Applies to future interactive shells without an inherited active Flox environment. Running shells are unchanged.",
            );
        }
        message::plain(
            "Manual shell integration: unverified. Use 'flox envs --active' to inspect this shell.",
        );
    }
}
