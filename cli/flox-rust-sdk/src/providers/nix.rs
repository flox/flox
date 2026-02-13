use std::fmt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

use serde::Deserialize;
use tracing::debug;

static NIX_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("NIX_BIN")
        .unwrap_or_else(|_| env!("NIX_BIN").to_string())
        .into()
});
pub const NIX_VERSION: &str = env!("NIX_VERSION");

/// Returns a `Command` for `nix` with a default set of features enabled.
pub fn nix_base_command() -> Command {
    let mut command = Command::new(&*NIX_BIN);
    command.args([
        "--option",
        "extra-experimental-features",
        "nix-command flakes",
    ]);
    command
}

/// Substituter and signing key configuration read from the host's nix config.
#[derive(Debug, Clone, Default)]
pub struct NixSubstituterConfig {
    pub substituters: Vec<String>,
    pub trusted_public_keys: Vec<String>,
}

/// Renders as a `NIX_CONFIG`-compatible string using `extra-*` variants
/// so values are appended to (not replace) existing nix.conf settings.
impl fmt::Display for NixSubstituterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut lines = Vec::new();
        if !self.substituters.is_empty() {
            lines.push(format!(
                "extra-substituters = {}",
                self.substituters.join(" ")
            ));
        }
        if !self.trusted_public_keys.is_empty() {
            lines.push(format!(
                "extra-trusted-public-keys = {}",
                self.trusted_public_keys.join(" ")
            ));
        }
        f.write_str(&lines.join("\n"))
    }
}

/// Raw shape of a single setting in `nix config show --json` output.
/// Each setting is `{ "value": [...], ... }` — we only need the value(s).
#[derive(Default, Deserialize)]
struct NixConfigSetting {
    #[serde(default)]
    value: Vec<String>,
}

/// Subset of `nix config show --json` we care about.
/// Unknown keys are discarded by serde's default behaviour.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct NixConfigJson {
    substituters: NixConfigSetting,
    trusted_public_keys: NixConfigSetting,
}

impl NixSubstituterConfig {
    /// Read the host's effective substituter config via `nix config show --json`.
    pub fn from_nix_config() -> Result<Self, NixSubstituterConfigError> {
        let mut command = nix_base_command();
        command.args(["config", "show", "--json"]);

        debug!(?command, "running nix config show");
        let output = command.output().map_err(NixSubstituterConfigError::Exec)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(NixSubstituterConfigError::Failed(stderr));
        }

        let config: NixConfigJson =
            serde_json::from_slice(&output.stdout).map_err(NixSubstituterConfigError::Parse)?;

        let result = Self {
            substituters: config.substituters.value,
            trusted_public_keys: config.trusted_public_keys.value,
        };
        debug!(?result, "nix substituter config");
        Ok(result)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NixSubstituterConfigError {
    #[error("failed to execute nix config show")]
    Exec(#[source] std::io::Error),
    #[error("nix config show failed: {0}")]
    Failed(String),
    #[error("failed to parse nix config show output")]
    Parse(#[source] serde_json::Error),
}

pub mod test_helpers {
    use std::path::PathBuf;

    use super::*;

    /// Returns a Nix store path that's known to exist.
    pub fn known_store_path() -> PathBuf {
        NIX_BIN.to_path_buf().canonicalize().unwrap()
    }
}
