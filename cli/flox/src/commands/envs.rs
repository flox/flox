use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt::Display;
use std::path::Path;

use anyhow::Result;
use bpaf::Bpaf;
use crossterm::style::Stylize;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::env_registry::{
    env_registry_path,
    read_environment_registry,
    EnvRegistry,
};
use flox_rust_sdk::models::environment::DotFlox;
use serde_json::json;
use tracing::instrument;

use super::{ActiveEnvironments, UninitializedEnvironment};
use crate::commands::activated_environments;
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Debug, Clone)]
#[bpaf(fallback(Mode::All))]
enum Mode {
    #[bpaf(long, hide)]
    All,
    /// Show only the active environments
    #[bpaf(long)]
    Active,
}

#[derive(Bpaf, Debug, Clone)]
pub struct Envs {
    #[bpaf(external(mode))]
    mode: Mode,
    /// Format output as JSON
    #[bpaf(long)]
    json: bool,
}

impl Envs {
    /// List all environments
    ///
    /// If `--json` is passed, dispatch to [Self::handle_json]
    ///
    /// If `--active` is passed, print only the active environments
    /// Always prints headers and formats the output.
    #[instrument(name = "envs", skip_all)]
    pub fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("envs");

        let active = activated_environments();

        match self.mode {
            Mode::Active => tracing::info_span!("active").in_scope(|| self.handle_active(active)),
            Mode::All => tracing::info_span!("all").in_scope(|| {
                let env_registry =
                    read_environment_registry(env_registry_path(&flox))?.unwrap_or_default();
                let registered = get_registered_environments(&env_registry);

                self.handle_all(active, registered)
            }),
        }
    }

    /// Print active environments only
    ///
    /// If `--json` is passed, print a JSON list with objects for each active environment.
    /// Otherwise, print a list of active environments.
    /// If no environments are active, print an appropriate message.
    fn handle_active(&self, active: ActiveEnvironments) -> Result<()> {
        if self.json {
            println!("{:#}", json!(active));
            return Ok(());
        }

        if active.last_active().is_none() {
            message::plain("No active environments");
            return Ok(());
        }

        message::created("Active environments:");
        let envs =
            indent::indent_all_by(2, DisplayEnvironments::new(active.iter(), true).to_string());
        println!("{envs}");

        Ok(())
    }

    /// Print all environments
    ///
    /// If `--json` is passed, print a JSON object with `active` and `inactive` keys.
    /// If any environments are active, print them first.
    /// Then print all inactive environments.
    /// If no environments are known to Flox, print an appropriate message.
    fn handle_all(
        &self,
        active: ActiveEnvironments,
        registered: impl Iterator<Item = UninitializedEnvironment>,
    ) -> Result<()> {
        let inactive = get_inactive_environments(registered, active.iter())?;

        if self.json {
            println!(
                "{:#}",
                json!({
                    "active": active,
                    "inactive": inactive,
                })
            );
            return Ok(());
        }

        if active.iter().next().is_none() && inactive.is_empty() {
            message::plain("No environments known to Flox");
        }

        if active.iter().next().is_some() {
            message::created("Active environments:");
            let envs =
                indent::indent_all_by(2, DisplayEnvironments::new(active.iter(), true).to_string());
            println!("{envs}");
        }

        if !inactive.is_empty() {
            message::plain("Inactive environments:");
            let envs = indent::indent_all_by(
                2,
                DisplayEnvironments::new(inactive.iter(), false).to_string(),
            );
            println!("{envs}");
        }

        Ok(())
    }
}

pub(crate) struct DisplayEnvironments<'a> {
    envs: Vec<&'a UninitializedEnvironment>,
    format_active: bool,
}

impl<'a> DisplayEnvironments<'a> {
    pub(crate) fn new(
        envs: impl IntoIterator<Item = &'a UninitializedEnvironment>,
        format_active: bool,
    ) -> Self {
        Self {
            envs: envs.into_iter().collect(),
            format_active,
        }
    }
}

impl<'a> Display for DisplayEnvironments<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let widest = self
            .envs
            .iter()
            .map(|env| format_description(env).len())
            .max()
            .unwrap_or(0);

        let mut envs = self.envs.iter();

        if self.format_active {
            let Some(first) = envs.next() else {
                return Ok(());
            };
            let first_formatted =
                format!("{:<widest$}  {}", first.name(), format_path(first.path())).bold();
            writeln!(f, "{first_formatted}")?;
        }

        for env in envs {
            writeln!(f, "{:<widest$}  {}", env.name(), format_path(env.path()))?;
        }

        Ok(())
    }
}

fn format_description(env: &UninitializedEnvironment) -> Cow<'_, str> {
    match env.bare_description() {
        Ok(desc) => desc.into(),
        Err(_) => "(unknown)".into(),
    }
}

fn format_path(path: Option<&Path>) -> Cow<'_, str> {
    path.map(|p| p.parent().unwrap_or(p).to_string_lossy())
        .unwrap_or_else(|| "(remote)".into())
}

fn get_registered_environments(
    registry: &EnvRegistry,
) -> impl Iterator<Item = UninitializedEnvironment> + '_ {
    registry.entries.iter().filter_map(|entry| {
        let path = entry.path.clone();
        let pointer = entry.latest_env()?.pointer.clone();

        // If we have a path registered that has since been deleted, skip it
        if !entry.path.exists() {
            return None;
        }

        Some(UninitializedEnvironment::DotFlox(DotFlox { path, pointer }))
    })
}

/// Get the list of environments that are not active
fn get_inactive_environments<'a>(
    available: impl IntoIterator<Item = UninitializedEnvironment>,
    active: impl IntoIterator<Item = &'a UninitializedEnvironment>,
) -> Result<BTreeSet<UninitializedEnvironment>> {
    // let active = activated_environments();

    let inactive = {
        let mut available = BTreeSet::from_iter(available);
        for active in active {
            available.remove(active);
        }
        available
    };

    Ok(inactive)
}
