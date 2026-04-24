use std::fmt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

use serde::Deserialize;
use tracing::debug;

/// Minimum Nix version that supports S3 multipart upload query parameters.
pub const NIX_MULTIPART_MIN_VERSION: semver::Version = semver::Version::new(2, 33, 0);

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

/// Raw shape of a single setting in `nix config show --json` output.
/// Each setting is `{ "value": [...], ... }` — we only need the value(s).
#[derive(Default, Deserialize)]
struct NixConfigSetting {
    #[serde(default)]
    value: Vec<String>,
}

/// Subset of `nix config show --json` we currently care about.
/// Unknown keys are discarded by serde's default behaviour.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct NixConfigJson {
    substituters: NixConfigSetting,
    trusted_public_keys: NixConfigSetting,
}

impl NixConfigJson {
    /// Read the host's effective nix config via `nix config show --json`.
    fn from_nix_config() -> Result<Self, NixSubstituterConfigError> {
        let mut command = nix_base_command();
        command.args(["config", "show", "--json"]);

        debug!(?command, "running nix config show");
        let output = command.output().map_err(NixSubstituterConfigError::Exec)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(NixSubstituterConfigError::Failed(stderr));
        }

        serde_json::from_slice(&output.stdout).map_err(NixSubstituterConfigError::Parse)
    }
}

/// Substituter and signing key configuration read from the host's nix config.
#[derive(Debug, Clone, Default)]
pub struct NixSubstituterConfig {
    pub substituters: Vec<String>,
    pub trusted_public_keys: Vec<String>,
}

impl NixSubstituterConfig {
    /// Read the host's effective substituter config.
    pub fn from_nix_config() -> Result<Self, NixSubstituterConfigError> {
        let result: Self = NixConfigJson::from_nix_config()?.into();
        debug!(?result, "nix substituter config");
        Ok(result)
    }
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

impl From<NixConfigJson> for NixSubstituterConfig {
    fn from(config: NixConfigJson) -> Self {
        Self {
            substituters: config.substituters.value,
            trusted_public_keys: config.trusted_public_keys.value,
        }
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

#[derive(Debug, thiserror::Error)]
pub enum NixVersionError {
    #[error("failed to execute nix --version")]
    Exec(#[source] std::io::Error),
    #[error("nix --version failed: {0}")]
    Failed(String),
    #[error("failed to parse nix version from output: {0}")]
    Parse(String),
}

/// Parse the version from `nix --version` output.
///
/// Expected format: `"nix (Nix) 2.31.2+1\n"`.
/// Development builds use: `"nix (Nix) 2.33.0pre20251224_e23983d"`.
///
/// The `+N` build metadata suffix is stripped because build metadata does
/// not affect semver precedence and simplifies parsing. The `preYYYYMMDD_hash`
/// dev suffix is not valid semver, so we strip it to extract the base version.
fn parse_nix_version_output(output: &str) -> Result<semver::Version, NixVersionError> {
    let version_str = output
        .trim()
        .strip_prefix("nix (Nix) ")
        .ok_or_else(|| NixVersionError::Parse(output.trim().to_string()))?;
    // Strip +N build suffix (e.g., "2.31.2+1" → "2.31.2")
    let clean = version_str.split('+').next().unwrap_or(version_str);
    // Strip preYYYYMMDD_hash dev suffix (e.g., "2.33.0pre20251224_e23983d" → "2.33.0")
    let clean = clean.split("pre").next().unwrap_or(clean);
    semver::Version::parse(clean).map_err(|_| NixVersionError::Parse(version_str.to_string()))
}

/// Detect the runtime Nix version by running `nix --version`.
///
/// This respects the `NIX_BIN` environment variable override, so it returns
/// the version of whichever Nix binary will actually be used for operations.
pub fn nix_runtime_version() -> Result<semver::Version, NixVersionError> {
    let output = Command::new(&*NIX_BIN)
        .arg("--version")
        .output()
        .map_err(NixVersionError::Exec)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(NixVersionError::Failed(stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_nix_version_output(&stdout)
}

pub mod test_helpers {
    use std::path::PathBuf;

    use super::*;

    /// Returns a Nix store path that's known to exist.
    pub fn known_store_path() -> PathBuf {
        NIX_BIN.to_path_buf().canonicalize().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_version() {
        let version = parse_nix_version_output("nix (Nix) 2.33.0").unwrap();
        assert_eq!(version, semver::Version::new(2, 33, 0));
    }

    #[test]
    fn parse_version_with_build_suffix() {
        let version = parse_nix_version_output("nix (Nix) 2.31.2+1").unwrap();
        assert_eq!(version, semver::Version::new(2, 31, 2));
    }

    #[test]
    fn parse_version_with_trailing_newline() {
        let version = parse_nix_version_output("nix (Nix) 2.33.1\n").unwrap();
        assert_eq!(version, semver::Version::new(2, 33, 1));
        assert!(version >= NIX_MULTIPART_MIN_VERSION);
    }

    #[test]
    fn parse_old_version_below_multipart_minimum() {
        let version = parse_nix_version_output("nix (Nix) 2.31.4").unwrap();
        assert!(version < NIX_MULTIPART_MIN_VERSION);
    }

    #[test]
    fn parse_prerelease_dev_build() {
        let version = parse_nix_version_output("nix (Nix) 2.33.0pre20251224_e23983d").unwrap();
        assert_eq!(version, semver::Version::new(2, 33, 0));
        assert!(version >= NIX_MULTIPART_MIN_VERSION);
    }

    #[test]
    fn parse_prerelease_with_build_metadata() {
        let version = parse_nix_version_output("nix (Nix) 2.33.0-rc.1+1").unwrap();
        assert_eq!(version, semver::Version::parse("2.33.0-rc.1").unwrap());
        // Pre-release versions compare below the release, so multipart
        // is not enabled for release candidates. This is intentional.
        assert!(version < NIX_MULTIPART_MIN_VERSION);
    }

    #[test]
    fn parse_garbage_input() {
        let result = parse_nix_version_output("garbage");
        assert!(matches!(result, Err(NixVersionError::Parse(_))));
    }

    #[test]
    fn parse_empty_version_string() {
        let result = parse_nix_version_output("nix (Nix) ");
        assert!(matches!(result, Err(NixVersionError::Parse(_))));
    }
}
