use std::borrow::Cow;
use std::collections::HashSet;
use std::io::Stderr;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::legacy_environment_ref::EnvironmentRef;
use flox_rust_sdk::providers::git::GitProvider;
use indoc::indoc;
use itertools::Itertools;
use log::{error, warn};
use once_cell::sync::Lazy;

pub mod colors;
mod completion;
pub mod dialog;
pub mod init;
pub mod logger;
pub mod metrics;

use regex::Regex;

use crate::utils::dialog::{Dialog, Select};

static NIX_IDENTIFIER_SAFE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[a-zA-Z0-9_-]+$"#).unwrap());
pub static TERMINAL_STDERR: Lazy<Mutex<Stderr>> = Lazy::new(|| Mutex::new(std::io::stderr()));

fn nix_str_safe(s: &str) -> Cow<str> {
    if NIX_IDENTIFIER_SAFE.is_match(s) {
        s.into()
    } else {
        format!("{s:?}").into()
    }
}

/// Resolve a single environment from a list of matches
///
/// - return an error if no matches were found
/// - return the match if there is only one
/// - start an interactive dialog if multiple matches were found
///   and a controlling tty was detected
pub async fn resolve_environment_ref<'flox, Git: GitProvider + 'static>(
    flox: &'flox Flox,
    subcommand: &str,
    environment_name: Option<&str>,
) -> Result<EnvironmentRef<'flox>> {
    let mut environment_refs = EnvironmentRef::find::<_, Git>(flox, environment_name).await?;
    match environment_refs.len() {
        0 => {
            bail!("No matching environments found");
        },
        1 => Ok(environment_refs.remove(0)),
        _ => {
            let mut sources: HashSet<Option<&Path>> = HashSet::new();

            for m in &environment_refs {
                if let EnvironmentRef::Project(p) = m {
                    sources.insert(Some(&p.workdir));
                } else {
                    sources.insert(None);
                }
            }

            let current_dir = std::env::current_dir()?;

            // Compile a list of choices for the user to choose from, and shorter choices for suggestions
            let mut choices: Vec<(String, &String)> = environment_refs
                .iter()
                .map(
                    // Format the results according to how verbose we have to be for disambiguation, only showing the flakeref or prefix when multiple are used
                    |m| {
                        let prefix: Cow<str> = match m {
                            EnvironmentRef::Named(_) if sources.len() > 1 => "Named - ".into(),
                            EnvironmentRef::Project(n) if sources.len() > 1 => {
                                let rel = pathdiff::diff_paths(&n.workdir, &current_dir)
                                    .ok_or_else(|| anyhow!("Project path should be absolute"))?;

                                if rel == Path::new("") {
                                    ". - ".into()
                                } else {
                                    format!("{} - ", rel.display()).into()
                                }
                            },
                            _ => "".into(),
                        };

                        let name = match m {
                            EnvironmentRef::Named(n) => &n.name,
                            EnvironmentRef::Project(p) => &p.name,
                        };

                        Ok((format!("{prefix}{name}"), name))
                    },
                )
                .collect::<Result<Vec<_>>>()?;

            if !Dialog::can_prompt() {
                error!(
                    indoc! {"
                    You must address a specific environment. For example with:

                      $ flox {subcommand} {first_choice},

                    The available environments are:
                    {choices_list}
                "},
                    subcommand = subcommand,
                    first_choice = choices.get(0).expect("Expected at least one choice").1,
                    choices_list = choices
                        .iter()
                        .map(|(long, _)| format!("  - {long}"))
                        .join("\n")
                );

                bail!("No terminal to prompt for environment choice");
            }

            // Prompt for the user to select match
            let dialog = Dialog {
                message: &format!("Select an environment for flox {subcommand}"),
                help_message: None,
                typed: Select {
                    options: choices.iter().cloned().map(|(long, _)| long).collect(),
                },
            };

            let (sel, _) = dialog
                .raw_prompt()
                .await
                .context("Failed to prompt for environment choice")?;

            let escaped = shell_escape::escape(choices.remove(sel).1.into()).into_owned();

            let environment_ref = environment_refs.remove(sel);

            warn!(
                "HINT: avoid selecting an environment next time with:\n  $ flox {subcommand} -e {escaped}",
            );

            Ok(environment_ref)
        },
    }
}
