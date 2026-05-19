/// Mocked plugin/integration commands for the Flox Agent prototype.
///
/// These commands return curated fake data to demonstrate the CLI surface
/// without implementing a real plugin loading system.  The FloxHub UI
/// mirrors this same list of integrations.
use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use serde::{Deserialize, Serialize};

use crate::utils::message;

/// Fake plugin registry — brand-neutral, no real partnerships implied.
static PLUGINS: &[PluginInfo] = &[
    PluginInfo {
        name: "cursor-adapter",
        description: "Cursor integration — forwards tool calls into your Cursor session",
        installed: false,
    },
    PluginInfo {
        name: "github-actions",
        description: "Run agents in CI — attach Flox Agent environments from GitHub Actions",
        installed: false,
    },
    PluginInfo {
        name: "linear-issues",
        description: "Auto-create Linear issues from agent runs and recap events",
        installed: false,
    },
    PluginInfo {
        name: "datadog-metrics",
        description: "Ship token usage and cost guardrail events to Datadog",
        installed: false,
    },
    PluginInfo {
        name: "slack-notify",
        description: "Post session recap to a Slack channel when an agent finishes",
        installed: false,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PluginInfo {
    name: &'static str,
    description: &'static str,
    installed: bool,
}

#[derive(Bpaf, Clone, Debug)]
pub enum PluginCommands {
    /// List available plugins and integrations
    #[bpaf(command)]
    List,

    /// Install a plugin by name
    #[bpaf(command)]
    Install {
        /// Plugin name (see 'flox plugin list')
        #[bpaf(positional("name"))]
        name: String,
    },

    /// Remove an installed plugin
    #[bpaf(command)]
    Remove {
        /// Plugin name (see 'flox plugin list')
        #[bpaf(positional("name"))]
        name: String,
    },
}

impl PluginCommands {
    pub fn handle(self, _flox: Flox) -> Result<()> {
        match self {
            PluginCommands::List => handle_list(),
            PluginCommands::Install { name } => handle_install(&name),
            PluginCommands::Remove { name } => handle_remove(&name),
        }
    }
}

fn handle_list() -> Result<()> {
    message::plain("Available plugins and integrations (Flox Agent prototype):\n");
    for plugin in PLUGINS {
        let status = if plugin.installed { "[installed]" } else { "[available]" };
        println!("  {:20}  {}  {}", plugin.name, status, plugin.description);
    }
    println!();
    message::plain(
        "Install a plugin with 'flox plugin install <name>'\nManage integrations on FloxHub at https://hub.flox.dev"
    );
    Ok(())
}

fn handle_install(name: &str) -> Result<()> {
    // Verify the plugin name is in our mocked list.
    if !PLUGINS.iter().any(|p| p.name == name) {
        anyhow::bail!(
            "Plugin '{}' not found.\nRun 'flox plugin list' to see available plugins.",
            name
        );
    }
    message::created(format!(
        "✨  Plugin '{}' installed (prototype — no real loading).\n  Configure it on FloxHub or in your environment manifest.",
        name
    ));
    Ok(())
}

fn handle_remove(name: &str) -> Result<()> {
    if !PLUGINS.iter().any(|p| p.name == name) {
        anyhow::bail!(
            "Plugin '{}' not found.\nRun 'flox plugin list' to see available plugins.",
            name
        );
    }
    message::plain(format!("🗑️  Plugin '{}' removed.", name));
    Ok(())
}
