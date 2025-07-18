use std::collections::BTreeSet;
use std::fmt::Display;
use std::path::Path;

use anyhow::Result;
use bpaf::Bpaf;
use crossterm::style::Stylize;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::env_registry::{EnvRegistry, garbage_collect};
use flox_rust_sdk::models::environment::{DotFlox, EnvironmentPointer, ManagedPointer};
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
                let env_registry = garbage_collect(&flox)?;
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

impl Display for DisplayEnvironments<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let widest = self
            .envs
            .iter()
            .map(|env| env.bare_description().len())
            .max()
            .unwrap_or(0);

        let mut envs = self.envs.iter();

        if self.format_active {
            let Some(first) = envs.next() else {
                return Ok(());
            };
            let first_formatted =
                format!("{:<widest$}  {}", first.name(), format_location(first)).bold();
            writeln!(f, "{first_formatted}")?;
        }

        for env in envs {
            writeln!(f, "{:<widest$}  {}", env.name(), format_location(env))?;
        }

        Ok(())
    }
}

/// Format the location (path and optional URL) of an environment.
fn format_location(env: &UninitializedEnvironment) -> String {
    match env {
        UninitializedEnvironment::DotFlox(DotFlox { path, pointer }) => match pointer {
            EnvironmentPointer::Path(_) => format_path(path),
            EnvironmentPointer::Managed(managed_pointer) => {
                format!("{} ({})", format_path(path), format_url(managed_pointer))
            },
        },
        UninitializedEnvironment::Remote(managed_pointer) => {
            format!("remote ({})", format_url(managed_pointer))
        },
    }
}

/// Format the URL of a FloxHub environment, logging any errors encountered.
fn format_url(pointer: &ManagedPointer) -> String {
    pointer.floxhub_url().map_or_else(
        |err| {
            // This is highly unlikely, given that most parse errors are
            // modifications to the base (proto, host, port) which can only be
            // done with `//` in the joined path and `EnvironmentOwner` and
            // `EnvironmentName` prevent slashes.
            tracing::warn!(?pointer, %err, "Failed to format URL for environment");
            "unknown".into()
        },
        |url| url.to_string(),
    )
}

fn format_path(path: &Path) -> String {
    path.parent().unwrap_or(path).to_string_lossy().to_string()
}

fn get_registered_environments(
    registry: &EnvRegistry,
) -> impl Iterator<Item = UninitializedEnvironment> + '_ {
    registry.entries.iter().filter_map(|entry| {
        let path = entry.path.clone();
        let pointer = entry.latest_env()?.pointer.clone();

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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::str::FromStr;

    use flox_rust_sdk::flox::{EnvironmentName, EnvironmentOwner, Floxhub};
    use flox_rust_sdk::models::environment::PathPointer;
    use indoc::formatdoc;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn display_environments() {
        let floxhub = Floxhub::new("https://hub.example.com".parse().unwrap(), None).unwrap();
        let owner = EnvironmentOwner::from_str("owner").unwrap();

        let path_env = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from("/envs/path/.flox"),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str("name_path").unwrap(),
            )),
        });

        let managed_env = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from("/envs/managed/.flox"),
            pointer: EnvironmentPointer::Managed(ManagedPointer::new(
                owner.clone(),
                EnvironmentName::from_str("name_managed").unwrap(),
                &floxhub,
            )),
        });

        let remote_env = UninitializedEnvironment::Remote(ManagedPointer::new(
            owner.clone(),
            EnvironmentName::from_str("name_remote").unwrap(),
            &floxhub,
        ));

        let envs = DisplayEnvironments {
            envs: vec![&path_env, &managed_env, &remote_env],
            format_active: false,
        };
        assert_eq!(envs.to_string(), formatdoc! {"
            name_path                   /envs/path
            name_managed                /envs/managed (https://hub.example.com/owner/name_managed)
            name_remote                 remote (https://hub.example.com/owner/name_remote)
        "});
    }
}
