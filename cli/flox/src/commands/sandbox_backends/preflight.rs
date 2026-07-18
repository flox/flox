//! Shared host-prerequisite checks for the sandbox backends.
//!
//! Every backend's `preflight` needs the same three primitives: locate a
//! required CLI on `PATH`, gate it against a minimum version with an
//! actionable message, and parse a `<HOST>:<PORT>` network endpoint from the
//! manifest. Collecting them here keeps the per-backend `preflight`
//! implementations to the parts that genuinely differ (which binaries, which
//! version floor, which install hint).

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use semver::Version;
use tracing::debug;

/// Resolve the first executable named `name` on `PATH`.
///
/// Returns the resolved path when a backend needs it for a follow-up check
/// (e.g. reporting the shadowing binary in a version-gate message); callers
/// that only need presence use [`binary_on_path`].
pub(crate) fn first_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// `true` if an executable named `name` is on `PATH`.
pub(crate) fn binary_on_path(name: &str) -> bool {
    first_on_path(name).is_some()
}

/// The default argument used to ask a CLI for its version. Most tools accept
/// `--version`; a backend whose CLI exposes the version via a subcommand
/// overrides [`CliVersionCheck::version_args`] instead.
pub(crate) const DEFAULT_VERSION_ARGS: &[&str] = &["--version"];

/// Per-backend copy for [`check_cli_version`].
///
/// The shared structure of every backend's version gate is identical; only the
/// display name, the quoted backend id, the upgrade instructions, and — for a
/// CLI without a `--version` flag — the version-query arguments differ.
pub(crate) struct CliVersionCheck<'a> {
    /// CLI display name used in the message (e.g. `OpenShell`, `Modal`).
    pub tool_name: &'a str,
    /// The backend id as it appears in `--sandbox-backend` (e.g. `openshell`).
    pub backend_id: &'a str,
    /// Minimum supported version.
    pub min_version: Version,
    /// Trailing lines with backend-specific upgrade guidance, appended after
    /// `Resolved binary: <path>`.
    pub upgrade_hint: &'a str,
    /// Arguments that make the CLI print its version. Defaults to
    /// [`DEFAULT_VERSION_ARGS`] (`--version`); a CLI without a `--version` flag
    /// (e.g. Docker Sandboxes' `sbx`, whose version is behind the `version`
    /// subcommand) sets this to its query, e.g. `&["version"]`.
    pub version_args: &'a [&'a str],
}

/// Gate a resolved CLI against a minimum version with an actionable message.
///
/// Runs `<path> <version_args…>` (defaulting to `--version`), parses the output
/// with [`parse_cli_version`], and bails when the version is below
/// `check.min_version`. A failed or unparseable version invocation skips the
/// gate (logged at debug) rather than blocking on an unknown output format —
/// the same tolerate-unparseable semantics each backend relied on before this
/// was shared.
pub(crate) fn check_cli_version(path: &Path, check: &CliVersionCheck<'_>) -> Result<()> {
    let output = std::process::Command::new(path)
        .args(check.version_args)
        .output();
    let raw = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => {
            debug!(
                path = %path.display(),
                tool = check.tool_name,
                "could not run the CLI version query; skipping version gate"
            );
            return Ok(());
        },
    };
    let Some(version) = parse_cli_version(&raw) else {
        debug!(
            path = %path.display(),
            tool = check.tool_name,
            output = raw.trim(),
            "unparseable CLI version output; skipping version gate"
        );
        return Ok(());
    };
    let min = &check.min_version;
    if version < *min {
        bail!(
            "{tool} CLI version {version} is too old for the '{backend}' sandbox backend (needs {min} or newer).\n\
             Resolved binary: {path}\n\
             {hint}",
            tool = check.tool_name,
            backend = check.backend_id,
            path = path.display(),
            hint = check.upgrade_hint,
        );
    }
    debug!(%version, tool = check.tool_name, "CLI version meets the minimum");
    Ok(())
}

/// Parse a semver version from `<cli> --version` output.
///
/// Splits on whitespace and colons (so both `openshell 0.0.82` and
/// `modal client version: 1.4.2` are handled) and returns the first token that
/// parses as a semver version, tolerating an optional leading `v`. Returns
/// `None` when no token parses.
pub(crate) fn parse_cli_version(output: &str) -> Option<Version> {
    output
        .split(|c: char| c.is_whitespace() || c == ':')
        .filter(|t| !t.is_empty())
        .find_map(|token| Version::parse(token.strip_prefix('v').unwrap_or(token)).ok())
}

/// Split a `<HOST>:<PORT>` endpoint and validate both halves.
///
/// The host charset is restricted to hostname characters plus a leading-label
/// wildcard (`*`); this doubles as injection protection for the single-quoted
/// scalars the policy/launcher artifacts embed the host in.
pub(crate) fn split_endpoint(endpoint: &str) -> Result<(String, u16)> {
    let invalid = || {
        anyhow::anyhow!(
            "Invalid sandbox network endpoint '{endpoint}'.\nWrite the endpoint as <HOST>:<PORT>, e.g. 'api.github.com:443'."
        )
    };
    let (host, port) = endpoint.rsplit_once(':').ok_or_else(invalid)?;
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '*'));
    if !host_ok {
        return Err(invalid());
    }
    let port: u16 = port.parse().map_err(|_| invalid())?;
    Ok((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_cli_version ─────────────────────────────────────────────────────

    #[test]
    fn version_parses_plain_cli_output() {
        assert_eq!(
            parse_cli_version("openshell 0.0.82"),
            Some(Version::new(0, 0, 82))
        );
        assert_eq!(parse_cli_version("1.4.2"), Some(Version::new(1, 4, 2)));
    }

    #[test]
    fn version_parses_labeled_output() {
        assert_eq!(
            parse_cli_version("modal client version: 1.4.2"),
            Some(Version::new(1, 4, 2))
        );
    }

    #[test]
    fn version_parses_v_prefixed_output() {
        assert_eq!(
            parse_cli_version("openshell v0.0.62"),
            Some(Version::new(0, 0, 62))
        );
        assert_eq!(
            parse_cli_version("modal v1.0.0"),
            Some(Version::new(1, 0, 0))
        );
    }

    #[test]
    fn version_unparseable_output_returns_none() {
        assert_eq!(parse_cli_version("not a version"), None);
        assert_eq!(parse_cli_version(""), None);
    }

    // ── split_endpoint ────────────────────────────────────────────────────────

    #[test]
    fn endpoint_splits_host_and_port() {
        assert_eq!(
            split_endpoint("api.github.com:443").unwrap(),
            ("api.github.com".to_string(), 443)
        );
    }

    #[test]
    fn endpoint_wildcard_host_is_accepted() {
        assert_eq!(
            split_endpoint("*.github.com:443").unwrap(),
            ("*.github.com".to_string(), 443)
        );
    }

    #[test]
    fn endpoint_without_port_is_rejected() {
        let err = split_endpoint("example.com").unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }

    #[test]
    fn endpoint_with_invalid_host_is_rejected() {
        let err = split_endpoint("bad host\nhost:443").unwrap_err();
        assert!(
            err.to_string().contains("Invalid sandbox network endpoint"),
            "got: {err}"
        );
    }

    #[test]
    fn endpoint_with_non_numeric_port_is_rejected() {
        let err = split_endpoint("example.com:https").unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }
}
