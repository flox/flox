use std::io::{Write, stdout};

use anyhow::Result;
use bpaf::Bpaf;
use crossterm::style::Stylize;
use crossterm::terminal;
use flox_manifest::interfaces::AsLatestSchema;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use serde::Serialize;
use tracing::instrument;

use super::{EnvironmentSelect, environment_select};
use crate::environment_subcommand_metric;
use crate::utils::markdown;
use crate::utils::message::stdout_supports_color;
use crate::utils::tracing::sentry_set_tag;

/// Show an environment's README along with a summary of what it provides.
///
/// Works on the environment in the current directory, an environment at
/// `--dir`, or a FloxHub environment with `-r <owner>/<name>`.
#[derive(Bpaf, Clone)]
pub struct Info {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Print the environment information as JSON
    #[bpaf(long)]
    json: bool,
}

/// The machine-readable shape of `flox info --json`.
#[derive(Debug, Serialize)]
struct EnvironmentInfo {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    readme: Option<String>,
}

impl Info {
    #[instrument(name = "info", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        let mut env = self
            .environment
            .detect_concrete_environment(&mut flox, "Show info for")
            .await?;
        environment_subcommand_metric!("info", env);
        sentry_set_tag("json", self.json.to_string());

        let name = env.name().to_string();
        let owner = match &env {
            ConcreteEnvironment::Path(_) => None,
            ConcreteEnvironment::Managed(env) => Some(env.owner().to_string()),
            ConcreteEnvironment::Remote(env) => Some(env.owner().to_string()),
        };
        let readme = env.readme(&flox)?;

        let manifest = env.manifest(&flox)?;
        let latest = manifest.as_latest_schema();
        let description = latest.description.clone();

        if self.json {
            let info = EnvironmentInfo {
                name,
                owner,
                description,
                readme,
            };
            let mut stdout = stdout();
            serde_json::to_writer_pretty(&mut stdout, &info)?;
            writeln!(stdout)?;
            return Ok(());
        }

        let rendered = render_human(RenderInput {
            name,
            owner,
            description,
            readme,
        });
        let mut stdout = stdout();
        write!(stdout, "{rendered}")?;
        Ok(())
    }
}

struct RenderInput {
    name: String,
    owner: Option<String>,
    description: Option<String>,
    readme: Option<String>,
}

fn render_human(input: RenderInput) -> String {
    let color = stdout_supports_color();
    let width = terminal::size()
        .map(|(cols, _)| cols as usize)
        .unwrap_or(80);
    // Render markdown a touch narrower than the terminal for readability.
    let content_width = width.saturating_sub(2).clamp(40, 100);

    let reference = match &input.owner {
        Some(owner) => format!("{owner}/{}", input.name),
        None => input.name.clone(),
    };

    let mut out = String::new();

    let title = if color {
        reference.clone().bold().magenta().to_string()
    } else {
        reference.clone()
    };
    out.push_str(&title);
    out.push('\n');

    if let Some(description) = &input.description {
        let line = if color {
            description.clone().grey().italic().to_string()
        } else {
            description.clone()
        };
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');

    match &input.readme {
        Some(readme) if !readme.trim().is_empty() => {
            out.push_str(&markdown::render(readme, content_width, color));
            out.push('\n');
        },
        _ => {
            let hint = match &input.owner {
                // Only local (path) environments can have a README added here.
                None => "This environment has no README yet. Add one with 'flox edit --readme'.",
                Some(_) => "This environment has no README yet.",
            };
            out.push_str(&dim(hint, color));
            out.push('\n');
        },
    }

    if let Some(owner) = &input.owner {
        out.push('\n');
        let next = format!(
            "Activate it with 'flox activate -r {owner}/{}'.",
            input.name
        );
        out.push_str(&dim(&next, color));
        out.push('\n');
    }

    out
}

fn dim(text: &str, color: bool) -> String {
    if color {
        text.to_string().dark_grey().to_string()
    } else {
        text.to_string()
    }
}
