use std::collections::BTreeSet;
use std::fmt::Display;
use std::path::Path;

use anyhow::Result;
use bpaf::Bpaf;
use crossterm::style::Stylize;
use flox_manifest::interfaces::AsLatestSchema;
use flox_manifest::{MANIFEST_FILENAME, Manifest};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::env_registry::{EnvRegistry, garbage_collect};
use flox_rust_sdk::models::environment::{
    DotFlox,
    ENV_DIR_NAME,
    EnvironmentPointer,
    ManagedPointer,
};
use serde_json::json;
use tracing::instrument;
use unicode_width::UnicodeWidthChar;

use super::UninitializedEnvironment;
use crate::subcommand_metric;
use crate::utils::active_environments::{ActiveEnvironments, activated_environments};
use crate::utils::markdown::first_line_plain;
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
        let max_width = message::terminal_width();

        let mut envs = self.envs.iter();

        if self.format_active {
            let Some(first) = envs.next() else {
                return Ok(());
            };
            let first_formatted =
                format!("{:<widest$}  {}", first.name(), format_location(first)).bold();
            writeln!(f, "{first_formatted}")?;
            write_description_line(f, first, max_width)?;
        }

        for env in envs {
            writeln!(f, "{:<widest$}  {}", env.name(), format_location(env))?;
            write_description_line(f, env, max_width)?;
        }

        Ok(())
    }
}

/// Write the environment's one-line description beneath its name, dimmed
/// and truncated to fit the terminal -- or nothing, when there's no
/// description to show.
fn write_description_line(
    f: &mut std::fmt::Formatter<'_>,
    env: &UninitializedEnvironment,
    max_width: usize,
) -> std::fmt::Result {
    let Some(description) = environment_description(env) else {
        return Ok(());
    };
    let first_line = first_line_plain(&description);
    if first_line.is_empty() {
        return Ok(());
    }
    writeln!(
        f,
        "{}",
        truncate_with_ellipsis(&first_line, max_width).dark_grey()
    )
}

/// Read the `description` field from an environment's manifest, for
/// display beneath its name in `flox envs`.
///
/// Best-effort: a missing, unreadable, or malformed manifest hides the
/// description line rather than failing `flox envs`.
///
/// Only path environments are read. Managed and remote environments
/// keep their manifest in a floxmeta git clone, and the only
/// local-only open path (`FloxMeta::open_local`) doesn't expose the
/// git handle a caller outside `flox-rust-sdk` would need to read a
/// generation's manifest without going through the full `open`, which
/// can fetch -- `flox envs` must never add a network call.
fn environment_description(env: &UninitializedEnvironment) -> Option<String> {
    let UninitializedEnvironment::DotFlox(DotFlox {
        path,
        pointer: EnvironmentPointer::Path(_),
    }) = env
    else {
        return None;
    };

    let manifest_path = path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME);
    let manifest = Manifest::read_typed(manifest_path).ok()?;
    let migrated = manifest.migrate(None).ok()?;
    migrated.as_latest_schema().description.clone()
}

/// Truncate `text` to `max_width` terminal columns, appending an
/// ellipsis when it doesn't fit. Width-aware (not byte- or
/// char-indexed) so a wide character at the boundary isn't cut in half.
fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let total_width: usize = text.chars().filter_map(|c| c.width()).sum();
    if total_width <= max_width {
        return text.to_string();
    }

    const ELLIPSIS: char = '…';
    let budget = max_width.saturating_sub(ELLIPSIS.width().unwrap_or(1));

    let mut truncated = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        truncated.push(c);
        used += w;
    }
    truncated.push(ELLIPSIS);
    truncated
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
    use flox_core::floxhub::Floxhub;
    use flox_rust_sdk::models::environment::PathPointer;
    use indoc::{formatdoc, indoc};
    use pretty_assertions::assert_eq;

    use super::*;

    /// A manifest body whose description opens with an ATX H1 and a
    /// second paragraph, so tests can assert both that the heading is
    /// stripped and that only the first line survives into the row.
    const DESCRIBED_MANIFEST_BODY: &str = indoc! {r#"
        description = """
        # My Env

        Body text."""
    "#};

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
            envs: vec![&path_env, &managed_env, &remote_env],
            format_active: false,
        };
        assert_eq!(envs.to_string(), formatdoc! {"
            name_path                  /envs/path
            name_managed               /envs/managed (https://hub.example.com/owner/name_managed)
            name_remote                remote (https://hub.example.com/owner/name_remote)
        "});
    }

    /// Create a `.flox/env/manifest.toml` under a fresh temp dir and
    /// return a path environment pointing at it, alongside the `TempDir`
    /// (whose drop deletes the directory, so it must outlive the env).
    fn path_env_with_manifest(
        name: &str,
        manifest_toml: &str,
    ) -> (UninitializedEnvironment, tempfile::TempDir) {
        let tempdir = tempfile::tempdir().unwrap();
        let env_dir = tempdir.path().join(".flox").join(ENV_DIR_NAME);
        std::fs::create_dir_all(&env_dir).unwrap();
        std::fs::write(env_dir.join(MANIFEST_FILENAME), manifest_toml).unwrap();

        let env = UninitializedEnvironment::DotFlox(DotFlox {
            path: tempdir.path().join(".flox"),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str(name).unwrap(),
            )),
        });
        (env, tempdir)
    }

    #[test]
    fn environment_description_reads_path_environment_manifest() {
        let manifest_toml =
            flox_manifest::test_helpers::with_latest_schema(DESCRIBED_MANIFEST_BODY);
        let (env, _tempdir) = path_env_with_manifest("described", &manifest_toml);

        assert_eq!(
            environment_description(&env).as_deref(),
            Some("# My Env\n\nBody text.")
        );
    }

    #[test]
    fn environment_description_is_none_without_a_description_field() {
        let manifest_toml = flox_manifest::test_helpers::with_latest_schema("");
        let (env, _tempdir) = path_env_with_manifest("undescribed", &manifest_toml);

        assert_eq!(environment_description(&env), None);
    }

    #[test]
    fn environment_description_is_none_for_a_missing_manifest() {
        let env = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from("/does/not/exist/.flox"),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str("missing").unwrap(),
            )),
        });

        assert_eq!(environment_description(&env), None);
    }

    /// Managed and remote environments are skipped entirely (see
    /// `environment_description`'s doc comment for why) -- confirmed
    /// here so a future change to that match doesn't silently start
    /// reading floxmeta and risk a network call.
    #[test]
    fn environment_description_is_none_for_managed_and_remote() {
        let floxhub = Floxhub::new("https://hub.example.com".parse().unwrap(), None, None).unwrap();
        let owner = EnvironmentOwner::from_str("owner").unwrap();

        let managed_env = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from("/envs/managed/.flox"),
            pointer: EnvironmentPointer::Managed(ManagedPointer::new(
                owner.clone(),
                EnvironmentName::from_str("name_managed").unwrap(),
                &floxhub,
            )),
        });
        let remote_env = UninitializedEnvironment::Remote(ManagedPointer::new(
            owner,
            EnvironmentName::from_str("name_remote").unwrap(),
            &floxhub,
        ));

        assert_eq!(environment_description(&managed_env), None);
        assert_eq!(environment_description(&remote_env), None);
    }

    #[test]
    fn display_environments_shows_stripped_truncated_description() {
        let manifest_toml =
            flox_manifest::test_helpers::with_latest_schema(DESCRIBED_MANIFEST_BODY);
        let (env, _tempdir) = path_env_with_manifest("described", &manifest_toml);

        let envs = DisplayEnvironments {
            envs: vec![&env],
            format_active: false,
        };
        let output = envs.to_string();

        // The row shows the Markdown-stripped first line, dimmed --
        // never the raw "# My Env" syntax, and never ANSI-rendered
        // Markdown (no heading color codes, just the dim escape).
        assert!(output.contains("My Env"), "{output:?}");
        assert!(!output.contains("# My Env"), "{output:?}");
        assert!(!output.contains("Body text"), "{output:?}");
    }

    #[test]
    fn display_environments_omits_description_line_when_absent() {
        let manifest_toml = flox_manifest::test_helpers::with_latest_schema("");
        let (env, _tempdir) = path_env_with_manifest("undescribed", &manifest_toml);

        let envs = DisplayEnvironments {
            envs: vec![&env],
            format_active: false,
        };
        assert_eq!(envs.to_string().lines().count(), 1);
    }

    #[test]
    fn truncate_with_ellipsis_leaves_short_text_unchanged() {
        assert_eq!(truncate_with_ellipsis("short", 80), "short");
    }

    #[test]
    fn truncate_with_ellipsis_truncates_long_text() {
        let text = "a description that is much too long to fit in the available width";
        let truncated = truncate_with_ellipsis(text, 20);

        assert!(truncated.ends_with('…'), "{truncated:?}");
        let width: usize = truncated.chars().filter_map(|c| c.width()).sum();
        assert!(width <= 20, "{truncated:?} has width {width}");
    }

    #[test]
    fn truncate_with_ellipsis_never_panics_on_wide_characters() {
        // CJK wide characters landing exactly at the truncation boundary.
        let text = "宽字符宽字符宽字符宽字符宽字符宽字符宽字符";
        for max_width in [0, 1, 2, 3, 5, 10] {
            let truncated = truncate_with_ellipsis(text, max_width);
            let width: usize = truncated.chars().filter_map(|c| c.width()).sum();
            assert!(width <= max_width.max(1), "{truncated:?} has width {width}");
        }
    }
}
