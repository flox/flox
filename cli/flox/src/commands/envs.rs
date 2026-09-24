use std::collections::BTreeSet;
use std::fmt::Display;
use std::path::Path;

use anyhow::Result;
use bpaf::Bpaf;
use crossterm::style::Stylize;
use flox_manifest::interfaces::AsLatestSchema;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::env_registry::{EnvRegistry, garbage_collect};
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{DotFlox, EnvironmentPointer, ManagedPointer};
use serde_json::json;
use tracing::instrument;

use super::UninitializedEnvironment;
use crate::subcommand_metric;
use crate::utils::active_environments::{
    ActiveEnvironment,
    ActiveEnvironments,
    activated_environments,
};
use crate::utils::markdown::{first_line_plain, truncate_to_width};
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
            Mode::Active => {
                tracing::info_span!("active").in_scope(|| self.handle_active(&flox, active))
            },
            Mode::All => tracing::info_span!("all").in_scope(|| {
                let env_registry = garbage_collect(&flox)?;
                let registered = get_registered_environments(&env_registry);

                self.handle_all(&flox, active, registered)
            }),
        }
    }

    /// Print active environments only
    ///
    /// If `--json` is passed, print a JSON list with objects for each active environment.
    /// Otherwise, print a list of active environments.
    /// If no environments are active, print an appropriate message.
    fn handle_active(&self, flox: &Flox, active: ActiveEnvironments) -> Result<()> {
        if self.json {
            let envs: Vec<_> = active
                .iter_full()
                .map(|env| active_environment_json(flox, env))
                .collect();
            println!("{:#}", json!(envs));
            return Ok(());
        }

        if active.last_active().is_none() {
            message::plain("No active environments");
            return Ok(());
        }

        message::created("Active environments:");
        let envs = indent::indent_all_by(
            2,
            DisplayEnvironments::new(flox, active.iter(), true).to_string(),
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
        flox: &Flox,
        active: ActiveEnvironments,
        registered: impl Iterator<Item = UninitializedEnvironment>,
    ) -> Result<()> {
        // Strip cache-checkout entries that back active remote environments
        // before computing the inactive set.  The registry entry itself is
        // preserved — GC pruning and `flox delete -r` depend on it — only the
        // display is filtered.
        let registered = registered.filter(|env| !is_cached_remote_backing(flox, env));
        let inactive = get_inactive_environments(registered, active.iter())?;

        if self.json {
            let active_json: Vec<_> = active
                .iter_full()
                .map(|env| active_environment_json(flox, env))
                .collect();
            let inactive_json: Vec<_> = inactive
                .iter()
                .map(|env| environment_json(flox, env))
                .collect();
            println!(
                "{:#}",
                json!({
                    "active": active_json,
                    "inactive": inactive_json,
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
                DisplayEnvironments::new(flox, active.iter(), true).to_string(),
            );
            println!("{envs}");
        }

        if !inactive.is_empty() {
            message::plain("Inactive environments:");
            let envs = indent::indent_all_by(
                2,
                DisplayEnvironments::new(flox, inactive.iter(), false).to_string(),
            );
            println!("{envs}");
        }

        Ok(())
    }
}

/// Serialize an active environment for `flox envs --json`, the same as its
/// derived `Serialize` impl (used to persist `$FLOX_ACTIVE_ENVIRONMENTS`),
/// plus a sibling `description` field carrying the *full* description
/// (never the truncated display line envs' non-JSON rows use).
fn active_environment_json(flox: &Flox, env: &ActiveEnvironment) -> serde_json::Value {
    with_description_field(flox, &env.environment, json!(env))
}

/// Serialize an inactive environment for `flox envs --json`, plus a
/// sibling `description` field; see [`active_environment_json`].
fn environment_json(flox: &Flox, env: &UninitializedEnvironment) -> serde_json::Value {
    with_description_field(flox, env, json!(env))
}

/// Insert a `description` field into `value` (expected to be a JSON
/// object), reading it via [`description_of`]. A read failure serializes
/// as `null` — matching "no description" — since `flox envs --json` must
/// not fail over one field.
fn with_description_field(
    flox: &Flox,
    env: &UninitializedEnvironment,
    mut value: serde_json::Value,
) -> serde_json::Value {
    if let serde_json::Value::Object(ref mut map) = value {
        map.insert("description".to_string(), json!(description_of(flox, env)));
    }
    value
}

/// The environment's `description`, read via
/// [`UninitializedEnvironment::migrated_manifest_without_lockfile`] --
/// absent for an environment whose manifest can't be located or read.
/// Which of those it was doesn't change what a listing shows, so they
/// collapse to `None` here.
fn description_of(flox: &Flox, env: &UninitializedEnvironment) -> Option<String> {
    let manifest = env.migrated_manifest_without_lockfile(flox)?;
    manifest.as_latest_schema().description.clone()
}

pub(crate) struct DisplayEnvironments<'a> {
    /// Each environment paired with its description, read once up front
    /// rather than in `fmt`, which cannot fail or do I/O.
    envs: Vec<(&'a UninitializedEnvironment, Option<String>)>,
    format_active: bool,
}

impl<'a> DisplayEnvironments<'a> {
    pub(crate) fn new(
        flox: &Flox,
        envs: impl IntoIterator<Item = &'a UninitializedEnvironment>,
        format_active: bool,
    ) -> Self {
        Self {
            envs: envs
                .into_iter()
                .map(|env| (env, description_of(flox, env)))
                .collect(),
            format_active,
        }
    }

    /// The same display with no descriptions looked up, for the one caller
    /// with no `Flox` to look them up with: `print_welcome_message` runs on
    /// the bare-command path, before `flox`'s top-level dispatch builds a
    /// `Flox`.
    pub(crate) fn without_descriptions(
        envs: impl IntoIterator<Item = &'a UninitializedEnvironment>,
        format_active: bool,
    ) -> Self {
        Self {
            envs: envs.into_iter().map(|env| (env, None)).collect(),
            format_active,
        }
    }
}

impl Display for DisplayEnvironments<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let widest = self
            .envs
            .iter()
            .map(|(env, _)| env.bare_description().len())
            .max()
            .unwrap_or(0);

        let mut envs = self.envs.iter();

        if self.format_active {
            let Some((first, description)) = envs.next() else {
                return Ok(());
            };
            let first_formatted =
                format!("{:<widest$}  {}", first.name(), format_location(first)).bold();
            writeln!(f, "{first_formatted}")?;
            write_description_row(f, description)?;
        }

        for (env, description) in envs {
            writeln!(f, "{:<widest$}  {}", env.name(), format_location(env))?;
            write_description_row(f, description)?;
        }

        Ok(())
    }
}

/// Print a dimmed, one-line, Markdown-stripped description row beneath an
/// environment's name/location line — never ANSI-rendered Markdown, which
/// would break a table row that's supposed to be exactly one line.
/// Environments without a description (or one that's empty after stripping
/// Markdown syntax) print nothing, leaving the row exactly as before this
/// field existed.
fn write_description_row(
    f: &mut std::fmt::Formatter<'_>,
    description: &Option<String>,
) -> std::fmt::Result {
    let Some(description) = description else {
        return Ok(());
    };
    let first_line = first_line_plain(description);
    if first_line.is_empty() {
        return Ok(());
    }
    // -2 for the two-space indent below, and again for the two-space
    // indent `indent::indent_all_by` applies to this whole block in the
    // caller, so the rendered row still fits one terminal line.
    let width = message::terminal_width().saturating_sub(4);
    writeln!(f, "  {}", truncate_to_width(&first_line, width).dim())
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

/// True when `env` is the managed-environment cache checkout that backs an
/// active remote environment.
///
/// Activating a [`RemoteEnvironment`] creates an inner [`ManagedEnvironment`]
/// under `~/.cache/flox/remote/<owner>/<name>/.flox` and registers it in the
/// env-registry.  That registration is load-bearing (GC pruning and
/// `flox delete -r` rely on it), but the cache entry must not appear as a
/// second inactive entry in `flox envs` alongside the active remote.
///
/// Detection is pointer-derived: the path must equal
/// `RemoteEnvironment::checkout_path(flox, pointer).join(".flox")`, not just
/// a prefix match on the cache dir.
fn is_cached_remote_backing(flox: &Flox, env: &UninitializedEnvironment) -> bool {
    let UninitializedEnvironment::DotFlox(DotFlox {
        path,
        pointer: EnvironmentPointer::Managed(mp),
    }) = env
    else {
        return false;
    };
    RemoteEnvironment::is_checkout_of(flox, path, mp)
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
    use flox_core::floxhub::Floxhub;
    use flox_rust_sdk::flox::test_helpers::flox_instance_with_optional_floxhub;
    use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
    use flox_rust_sdk::models::environment::{DOT_FLOX, PathPointer};
    use indoc::formatdoc;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn display_environments() {
        let floxhub = Floxhub::new("https://hub.example.com".parse().unwrap(), None, None).unwrap();
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
            envs: vec![(&path_env, None), (&managed_env, None), (&remote_env, None)],
            format_active: false,
        };
        assert_eq!(envs.to_string(), formatdoc! {"
            name_path                  /envs/path
            name_managed               /envs/managed (https://hub.example.com/owner/name_managed)
            name_remote                remote (https://hub.example.com/owner/name_remote)
        "});
    }

    #[test]
    fn display_environments_with_description_shows_dimmed_row() {
        let path_env = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from("/envs/path/.flox"),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str("name_path").unwrap(),
            )),
        });

        let envs = DisplayEnvironments {
            envs: vec![(&path_env, Some("# My Title\n\nBody.".to_string()))],
            format_active: false,
        };
        let rendered = envs.to_string();
        assert!(
            rendered.contains("My Title"),
            "expected the stripped title on its own row: {rendered}"
        );
        assert!(
            !rendered.contains("# My Title"),
            "the ATX marker must not survive into a table row: {rendered}"
        );
    }

    #[test]
    fn display_environments_without_description_renders_unchanged() {
        let path_env = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from("/envs/path/.flox"),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str("name_path").unwrap(),
            )),
        });

        let envs = DisplayEnvironments {
            envs: vec![(&path_env, None)],
            format_active: false,
        };
        assert_eq!(envs.to_string(), formatdoc! {"
            name_path  /envs/path
        "});
    }

    /// A `DotFlox::Managed` whose path matches the remote-environment cache
    /// checkout is identified as a backing entry.
    #[test]
    fn is_cached_remote_backing_true_for_cache_checkout() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (flox, _tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let name = EnvironmentName::from_str("myenv").unwrap();
        let pointer = ManagedPointer::new(owner, name, &flox.floxhub);
        let checkout_dot_flox = RemoteEnvironment::checkout_path(&flox, &pointer).join(DOT_FLOX);

        let env = UninitializedEnvironment::DotFlox(DotFlox {
            path: checkout_dot_flox,
            pointer: EnvironmentPointer::Managed(pointer),
        });
        assert!(is_cached_remote_backing(&flox, &env));
    }

    /// A plain managed env at an arbitrary path is not a backing entry.
    #[test]
    fn is_cached_remote_backing_false_for_arbitrary_managed_path() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (flox, _tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let name = EnvironmentName::from_str("myenv").unwrap();
        let pointer = ManagedPointer::new(owner, name, &flox.floxhub);

        let env = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from("/projects/myenv/.flox"),
            pointer: EnvironmentPointer::Managed(pointer),
        });
        assert!(!is_cached_remote_backing(&flox, &env));
    }

    /// A `DotFlox::Path` (not a managed pointer) is never a backing entry.
    #[test]
    fn is_cached_remote_backing_false_for_path_pointer() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (flox, _tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let name = EnvironmentName::from_str("myenv").unwrap();

        let env = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from("/projects/myenv/.flox"),
            pointer: EnvironmentPointer::Path(PathPointer::new(name)),
        });
        assert!(!is_cached_remote_backing(&flox, &env));
    }
}
