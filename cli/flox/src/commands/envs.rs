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

use super::UninitializedEnvironment;
use crate::commands::recap::persistent_marker_path;
use crate::subcommand_metric;
use crate::utils::active_environments::{ActiveEnvironments, activated_environments};
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
        let cache_dir = flox.cache_dir.clone();

        match self.mode {
            Mode::Active => {
                tracing::info_span!("active").in_scope(|| self.handle_active(active, cache_dir))
            },
            Mode::All => tracing::info_span!("all").in_scope(|| {
                let env_registry = garbage_collect(&flox)?;
                let registered = get_registered_environments(&env_registry);

                self.handle_all(active, registered, cache_dir)
            }),
        }
    }

    /// Print active environments only
    ///
    /// If `--json` is passed, print a JSON list with objects for each active environment.
    /// Otherwise, print a list of active environments.
    /// If no environments are active, print an appropriate message.
    fn handle_active(
        &self,
        active: ActiveEnvironments,
        cache_dir: std::path::PathBuf,
    ) -> Result<()> {
        if self.json {
            println!("{:#}", json!(active));
            return Ok(());
        }

        if active.last_active().is_none() {
            message::plain("No active environments");
            return Ok(());
        }

        message::created("Active environments:");
        let envs = indent::indent_all_by(
            2,
            DisplayEnvironments::new(active.iter(), true)
                .with_cache_dir(cache_dir)
                .to_string(),
        );
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
        cache_dir: std::path::PathBuf,
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
            let envs = indent::indent_all_by(
                2,
                DisplayEnvironments::new(active.iter(), true)
                    .with_cache_dir(cache_dir.clone())
                    .to_string(),
            );
            println!("{envs}");
        }

        if !inactive.is_empty() {
            message::plain("Inactive environments:");
            let envs = indent::indent_all_by(
                2,
                DisplayEnvironments::new(inactive.iter(), false)
                    .with_cache_dir(cache_dir)
                    .to_string(),
            );
            println!("{envs}");
        }

        Ok(())
    }
}

pub(crate) struct DisplayEnvironments<'a> {
    envs: Vec<&'a UninitializedEnvironment>,
    format_active: bool,
    /// Cache dir used to locate stable persistent markers.
    /// When None, persistent tags are not shown.
    cache_dir: Option<std::path::PathBuf>,
}

impl<'a> DisplayEnvironments<'a> {
    pub(crate) fn new(
        envs: impl IntoIterator<Item = &'a UninitializedEnvironment>,
        format_active: bool,
    ) -> Self {
        Self {
            envs: envs.into_iter().collect(),
            format_active,
            cache_dir: None,
        }
    }

    pub(crate) fn with_cache_dir(mut self, cache_dir: std::path::PathBuf) -> Self {
        self.cache_dir = Some(cache_dir);
        self
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
            let persistent_tag = self.persistent_tag_for(first).unwrap_or_default();
            let first_formatted = format!(
                "{:<widest$}  {}{}",
                first.name(),
                format_location(first),
                persistent_tag
            )
            .bold();
            writeln!(f, "{first_formatted}")?;
        }

        for env in envs {
            let persistent_tag = self.persistent_tag_for(env).unwrap_or_default();
            writeln!(
                f,
                "{:<widest$}  {}{}",
                env.name(),
                format_location(env),
                persistent_tag
            )?;
        }

        Ok(())
    }
}

impl DisplayEnvironments<'_> {
    /// Return the `[persistent]` tag string if a stable persistent marker
    /// exists for this environment, otherwise return `None`.
    ///
    /// The marker is written to `{cache_dir}/agent/persistent-markers/{hash}`
    /// by `flox activate --persistent` so it survives shell exit.
    fn persistent_tag_for(&self, env: &UninitializedEnvironment) -> Option<String> {
        let cache_dir = self.cache_dir.as_ref()?;
        let dot_flox_path = match env {
            UninitializedEnvironment::DotFlox(DotFlox { path, .. }) => path,
            // Remote envs are not locally persistent.
            UninitializedEnvironment::Remote(_) => return None,
        };
        let marker = persistent_marker_path(cache_dir, dot_flox_path);
        if marker.exists() {
            Some("  [persistent]".to_string())
        } else {
            None
        }
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

    use flox_core::data::environment_ref::{EnvironmentName, EnvironmentOwner};
    use flox_rust_sdk::flox::Floxhub;
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
            cache_dir: None,
        };
        assert_eq!(envs.to_string(), formatdoc! {"
            name_path                  /envs/path
            name_managed               /envs/managed (https://hub.example.com/owner/name_managed)
            name_remote                remote (https://hub.example.com/owner/name_remote)
        "});
    }

    /// `persistent_tag_for` returns `None` when no marker file exists and
    /// `Some("  [persistent]")` when `persistent_marker_path` points to an
    /// existing file — regardless of whether the activation is active or not.
    #[test]
    fn persistent_tag_for_marker_present_and_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let cache_dir = tmp.path().to_path_buf();

        let dot_flox_path = PathBuf::from("/envs/myenv/.flox");

        let path_env = UninitializedEnvironment::DotFlox(DotFlox {
            path: dot_flox_path.clone(),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str("myenv").unwrap(),
            )),
        });

        let display = DisplayEnvironments {
            envs: vec![&path_env],
            format_active: false,
            cache_dir: Some(cache_dir.clone()),
        };

        // No marker yet — tag should be absent.
        assert_eq!(display.persistent_tag_for(&path_env), None);

        // Write the stable marker the way activate does.
        let marker =
            crate::commands::recap::persistent_marker_path(&cache_dir, &dot_flox_path);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, b"1").unwrap();

        // Marker present — tag should appear.
        assert_eq!(
            display.persistent_tag_for(&path_env),
            Some("  [persistent]".to_string())
        );
    }
}
