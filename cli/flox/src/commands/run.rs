use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_catalog::{ClientTrait, SearchResults};
use flox_core::data::environment_ref::EnvironmentName;
use flox_manifest::raw::PackageToInstall;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::path_environment::PathEnvironment;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment, PathPointer};
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::EnvironmentSelect;
use super::activate::{Activate, CommandSelect};
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Select};
use crate::utils::message;

/// Run a binary from a package without installing it into an environment.
///
/// Looks up the binary name in the Flox catalog to find which package provides
/// it, creates a temporary environment with that package, and executes the
/// binary. The environment is cleaned up after the command exits.
///
/// When multiple packages provide the same binary, Flox prompts you to choose.
/// Your choice is remembered for future invocations. Use --reselect to pick
/// again, or --package to force a specific package.
#[derive(Bpaf, Clone, Debug)]
pub struct Run {
    /// Use a specific package instead of looking up the binary
    #[bpaf(long("package"), short('p'), argument("PKG"))]
    pub package: Option<String>,

    /// Clear the cached package choice and re-prompt
    #[bpaf(long("reselect"))]
    pub reselect: bool,

    /// The binary to run (e.g. "readelf", "jq", "vi")
    #[bpaf(positional("binary"))]
    pub binary: String,

    /// Arguments passed to the binary (after --)
    #[bpaf(positional("args"), strict, many)]
    pub args: Vec<String>,
}

// ---------------------------------------------------------------------------
// Cache types
// ---------------------------------------------------------------------------

/// User-scoped cache for binary → package choices.
///
/// Persisted as TOML at `state_dir/binary_preferences.toml`.
/// Mirrors the `trusted_environments` pattern in `flox.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BinaryPreferences {
    /// Map from binary name to chosen package attr_path.
    #[serde(default)]
    pub choices: HashMap<String, String>,
}

impl BinaryPreferences {
    /// Load from disk, returning a default empty cache if file does not exist.
    pub fn load(state_dir: &Path) -> Result<Self> {
        let path = preferences_path(state_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read binary preferences from {path:?}"))?;
        let prefs = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse binary preferences from {path:?}"))?;
        Ok(prefs)
    }

    /// Persist to disk atomically.
    pub fn save(&self, state_dir: &Path) -> Result<()> {
        let path = preferences_path(state_dir);
        std::fs::create_dir_all(state_dir)
            .with_context(|| format!("Failed to create state directory {state_dir:?}"))?;
        let contents =
            toml::to_string(self).context("Failed to serialize binary preferences")?;
        // Write to a temp file and rename for atomicity.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &contents)
            .with_context(|| format!("Failed to write binary preferences to {tmp:?}"))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("Failed to rename {tmp:?} to {path:?}"))?;
        Ok(())
    }

    /// Get the cached choice for a binary, if any.
    pub fn get(&self, binary: &str) -> Option<&str> {
        self.choices.get(binary).map(|s| s.as_str())
    }

    /// Set a choice and persist it.
    pub fn set_and_save(&mut self, binary: &str, pkg: &str, state_dir: &Path) -> Result<()> {
        self.choices.insert(binary.to_string(), pkg.to_string());
        self.save(state_dir)
    }

    /// Clear the choice for a binary and persist.
    pub fn clear_and_save(&mut self, binary: &str, state_dir: &Path) -> Result<()> {
        self.choices.remove(binary);
        self.save(state_dir)
    }
}

fn preferences_path(state_dir: &Path) -> PathBuf {
    state_dir.join("binary_preferences.toml")
}

// ---------------------------------------------------------------------------
// Package candidate type
// ---------------------------------------------------------------------------

/// A candidate package that provides the requested binary.
#[derive(Debug, Clone)]
pub struct PackageCandidate {
    pub attr_path: String,
    pub pname: String,
    pub description: Option<String>,
    pub version: Option<String>,
}

impl std::fmt::Display for PackageCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Show attr_path; fall back to pname only if they differ (e.g., nested attrs).
        write!(f, "{}", self.attr_path)?;
        if self.pname != self.attr_path {
            write!(f, " ({})", self.pname)?;
        } else if let Some(ref ver) = self.version {
            write!(f, " ({})", ver)?;
        }
        if let Some(ref desc) = self.description {
            // Truncate long descriptions for the menu.
            let truncated = if desc.len() > 60 {
                format!("{:.57}...", desc)
            } else {
                desc.clone()
            };
            write!(f, " — {}", truncated)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Binary lookup
// ---------------------------------------------------------------------------

/// Look up packages that provide the given binary via the catalog.
///
/// Because the `by-binary` endpoint may not exist yet, we fall back to
/// a search-based heuristic: search for the binary name and filter
/// results where `pname` or the last segment of `attr_path` matches.
///
/// When the real endpoint is added to the catalog API and codegen, replace
/// the body of this function with a direct API call.
pub async fn lookup_binary_candidates(
    binary: &str,
    flox: &Flox,
) -> Result<Vec<PackageCandidate>> {
    // Try the dedicated by-binary endpoint first.
    // The method returns Err if the endpoint doesn't exist or returns an
    // error, in which case we fall back to search.
    match flox
        .catalog_client
        .packages_by_binary(binary, flox.system.clone().try_into()?)
        .await
    {
        Ok(by_binary) if !by_binary.is_empty() => {
            debug!(
                binary,
                count = by_binary.len(),
                "found candidates via by-binary endpoint"
            );
            let candidates = by_binary
                .into_iter()
                .map(|p| PackageCandidate {
                    attr_path: p.attr_path,
                    pname: p.pname,
                    description: p.description,
                    version: p.version,
                })
                .collect();
            return Ok(candidates);
        },
        Ok(_) => {
            debug!(
                binary,
                "by-binary endpoint returned empty results, falling back to search"
            );
        },
        Err(e) => {
            debug!(
                binary,
                error = %e,
                "by-binary endpoint unavailable, falling back to search"
            );
        },
    }

    // Fallback: search for the binary name and heuristically filter.
    let limit = std::num::NonZeroU8::new(20);
    let results: SearchResults = flox
        .catalog_client
        .search(binary, flox.system.clone().try_into()?, limit)
        .await
        .with_context(|| format!("Failed to search catalog for binary '{binary}'"))?;

    let candidates: Vec<PackageCandidate> = results
        .results
        .into_iter()
        .filter(|pkg| {
            // Keep results where the last attr_path segment or pname match the binary.
            let last_segment = pkg.attr_path.rsplit('.').next().unwrap_or(&pkg.attr_path);
            last_segment == binary || pkg.pname == binary
        })
        .map(|pkg| PackageCandidate {
            attr_path: pkg.attr_path,
            pname: pkg.pname,
            description: pkg.description,
            version: pkg.version,
        })
        .collect();

    Ok(candidates)
}

// ---------------------------------------------------------------------------
// Disambiguation
// ---------------------------------------------------------------------------

/// Choose a package from a list of candidates.
///
/// - If stdin+stderr are TTYs: show an interactive prompt.
/// - Otherwise: return None (caller should error with helpful message).
pub async fn choose_package_interactive<'a>(
    binary: &'a str,
    candidates: &'a [PackageCandidate],
) -> Result<Option<&'a PackageCandidate>> {
    if !Dialog::<()>::can_prompt() {
        return Ok(None);
    }

    let options = candidates.to_vec();
    let message = format!(
        "Multiple packages provide '{}'. Which would you like to use?",
        binary
    );

    let choice = Dialog {
        message: &message,
        help_message: Some("Use arrow keys to select, Enter to confirm"),
        typed: Select { options },
    }
    .prompt()
    .await
    .context("Prompt interrupted")?;

    // Find the chosen candidate by attr_path
    let chosen = candidates
        .iter()
        .find(|c| c.attr_path == choice.attr_path)
        .expect("chosen candidate must be in the list");

    Ok(Some(chosen))
}

/// Build a helpful error message for ambiguous binary in non-interactive mode.
fn non_interactive_ambiguity_error(binary: &str, candidates: &[PackageCandidate]) -> String {
    let list = candidates
        .iter()
        .map(|c| format!("  - {}", c.attr_path))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Multiple packages provide '{binary}' and no cached choice exists.\n\
         Cannot prompt in non-interactive mode.\n\n\
         Packages that provide '{binary}':\n{list}\n\n\
         Use 'flox run --package <pkg> {binary}' to specify a package.",
    )
}

/// Build a helpful error message for binary not found.
fn not_found_error(binary: &str) -> String {
    format!(
        "No packages found that provide the binary '{binary}'.\n\
         Try 'flox search {binary}' to find related packages, then use\n\
         'flox run --package <pkg> {binary}' to run it explicitly.",
    )
}

// ---------------------------------------------------------------------------
// Main handler
// ---------------------------------------------------------------------------

impl Run {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("run");

        let state_dir = &config.flox.state_dir;
        let binary = &self.binary;

        // Determine which package attr_path to use.
        let pkg_attr_path = if let Some(ref pkg) = self.package {
            // Explicit --package: use it, update cache.
            debug!(binary, pkg, "using explicit --package override");
            let mut prefs = BinaryPreferences::load(state_dir)?;
            prefs.set_and_save(binary, pkg, state_dir)?;
            pkg.clone()
        } else {
            // Binary-first: look up the binary in the catalog.
            let mut prefs = BinaryPreferences::load(state_dir)?;

            // --reselect: clear cached choice before proceeding.
            if self.reselect {
                debug!(binary, "clearing cached choice due to --reselect");
                if Dialog::<()>::can_prompt() {
                    prefs.clear_and_save(binary, state_dir)?;
                } else {
                    bail!(
                        "--reselect requires an interactive terminal.\n\
                         Use 'flox run --package <pkg> {binary}' to specify a package.",
                    );
                }
            }

            // Check cache first (unless --reselect cleared it).
            if let Some(cached) = prefs.get(binary) {
                debug!(binary, pkg = cached, "using cached package choice");
                cached.to_string()
            } else {
                // Look up candidates.
                let candidates = lookup_binary_candidates(binary, &flox).await?;

                match candidates.len() {
                    0 => {
                        bail!("{}", not_found_error(binary));
                    },
                    1 => {
                        // Single candidate: use it and cache silently.
                        let pkg = &candidates[0].attr_path;
                        debug!(binary, pkg, "single candidate found, using it");
                        prefs.set_and_save(binary, pkg, state_dir)?;
                        pkg.clone()
                    },
                    _ => {
                        // Multiple candidates: disambiguate.
                        message::plain(format!(
                            "Multiple packages provide '{binary}'."
                        ));

                        let chosen =
                            choose_package_interactive(binary, &candidates).await?;

                        match chosen {
                            Some(candidate) => {
                                let pkg = &candidate.attr_path;
                                prefs.set_and_save(binary, pkg, state_dir)?;
                                pkg.clone()
                            },
                            None => {
                                bail!(
                                    "{}",
                                    non_interactive_ambiguity_error(binary, &candidates)
                                );
                            },
                        }
                    },
                }
            }
        };

        debug!(
            binary,
            pkg = %pkg_attr_path,
            "creating temporary environment for flox run"
        );

        // Parse the package to install.
        let package_spec = pkg_attr_path.clone();
        let package = PackageToInstall::parse(&flox.system, &package_spec)
            .context("Failed to parse package specification")?;

        // Only support catalog packages for now.
        if !matches!(package, PackageToInstall::Catalog(_)) {
            bail!(
                "flox run currently only supports catalog packages.\n\
                 Flake references and store paths are not supported."
            );
        }

        // Create a temp directory for the ephemeral environment.
        let run_temp_dir = flox.temp_dir.join("flox-run");
        std::fs::create_dir_all(&run_temp_dir)
            .context("Failed to create temporary directory for flox run")?;

        let env_name =
            EnvironmentName::from_str("run-temp").expect("'run-temp' is a valid environment name");
        let pointer = PathPointer::new(env_name);

        let mut path_env = PathEnvironment::init_bare(pointer, &run_temp_dir, &flox)
            .context("Failed to create temporary environment")?;

        debug!(
            "Installing package '{}' into temporary environment",
            package_spec
        );

        path_env
            .install(&[package], &flox)
            .with_context(|| {
                format!("Failed to install package '{}'", package_spec)
            })?;

        let mut concrete_environment = ConcreteEnvironment::Path(path_env);

        // Resolve the binary in the built environment.
        let rendered = concrete_environment
            .rendered_env_links(&flox)
            .context("Failed to get environment paths")?;
        let bin_dir = rendered.runtime.join("bin");

        // The binary name is the user-provided name; try to find it in the
        // installed package, falling back to the resolution heuristics.
        let resolved_binary = if bin_dir.is_dir() && bin_dir.join(binary).exists() {
            // Exact match.
            binary.clone()
        } else {
            resolve_binary(binary, &bin_dir, &pkg_attr_path)?
        };

        // Build the exec command: [binary_name, args...]
        let mut exec_args = vec![resolved_binary.clone()];
        exec_args.extend(self.args.clone());

        // Reuse the Activate flow — same pattern as services start.
        Activate {
            environment: EnvironmentSelect::Dir(run_temp_dir),
            trust: false,
            print_script: false,
            start_services: false,
            mode: None,
            generation: None,
            command: Some(CommandSelect::ExecCommand {
                command: resolved_binary,
                args: self.args,
            }),
        }
        .activate(
            config,
            flox,
            concrete_environment,
            flox_core::activate::context::InvocationType::ExecCommand(exec_args),
            Vec::new(), // no services
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Binary resolution helpers (kept from prototype)
// ---------------------------------------------------------------------------

/// List binary names available in a bin directory.
fn list_binaries(bin_dir: &Path) -> Vec<String> {
    let mut bins: Vec<String> = std::fs::read_dir(bin_dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type()
                .map(|ft| ft.is_file() || ft.is_symlink())
                .unwrap_or(false)
        })
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    bins.sort();
    bins
}

/// Resolve the best binary to run from a package's bin directory.
///
/// Strategy:
/// 1. If the name exists in bin/, use it (e.g. "cowsay" → "cowsay")
/// 2. If only one binary exists, use it
/// 3. If the name is a prefix of exactly one binary, use it
///    (e.g. "python3" → "python3.11")
/// 4. If a binary is a prefix of the name, use it
///    (e.g. "nodejs" → "node")
/// 5. Otherwise, error with the list of available binaries
pub fn resolve_binary(
    derived_name: &str,
    bin_dir: &Path,
    package_spec: &str,
) -> Result<String> {
    // Exact match
    if bin_dir.join(derived_name).exists() {
        return Ok(derived_name.to_string());
    }

    // No bin directory or empty — fall through to exec (let it fail naturally)
    if !bin_dir.is_dir() {
        return Ok(derived_name.to_string());
    }

    let available = list_binaries(bin_dir);
    if available.is_empty() {
        return Ok(derived_name.to_string());
    }

    // Only one binary — use it
    if available.len() == 1 {
        let bin = available[0].clone();
        debug!(
            "Binary '{}' not found, using only available binary '{}'",
            derived_name, bin
        );
        return Ok(bin);
    }

    // Derived name is a prefix of exactly one binary (python3 → python3.11)
    let prefix_matches: Vec<&String> = available
        .iter()
        .filter(|b| b.starts_with(derived_name))
        .collect();
    if prefix_matches.len() == 1 {
        let bin = prefix_matches[0].clone();
        debug!(
            "Binary '{}' not found, using prefix match '{}'",
            derived_name, bin
        );
        return Ok(bin);
    }

    // A binary is a prefix of the derived name (nodejs → node)
    let reverse_matches: Vec<&String> = available
        .iter()
        .filter(|b| derived_name.starts_with(b.as_str()))
        .collect();
    if reverse_matches.len() == 1 {
        let bin = reverse_matches[0].clone();
        debug!(
            "Binary '{}' not found, using reverse prefix match '{}'",
            derived_name, bin
        );
        return Ok(bin);
    }

    bail!(
        "Binary '{derived_name}' not found in package '{package_spec}'.\n\
         Try: flox run --package <PKG> {derived_name} -- ...\n\n\
         Available binaries: {}",
        available.join(", ")
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ----- derive_binary_name -----

    /// Derive a binary name from a package specification string.
    ///
    /// This utility is preserved for testing purposes; in production,
    /// the binary name is always provided directly by the user.
    fn derive_binary_name(package_spec: &str) -> String {
        package_spec
            .split('@')
            .next()
            .unwrap()
            .split('^')
            .next()
            .unwrap()
            .rsplit('.')
            .next()
            .unwrap()
            .to_string()
    }

    #[test]
    fn test_derive_binary_name_simple() {
        assert_eq!(derive_binary_name("cowsay"), "cowsay");
    }

    #[test]
    fn test_derive_binary_name_with_version() {
        assert_eq!(derive_binary_name("python@3.11"), "python");
    }

    #[test]
    fn test_derive_binary_name_nested_path() {
        assert_eq!(derive_binary_name("python3Packages.numpy"), "numpy");
    }

    #[test]
    fn test_derive_binary_name_with_outputs() {
        assert_eq!(derive_binary_name("curl^bin,man"), "curl");
    }

    #[test]
    fn test_derive_binary_name_version_and_path() {
        assert_eq!(derive_binary_name("python3Packages.numpy@1.24"), "numpy");
    }

    // ----- resolve_binary -----

    #[test]
    fn test_resolve_binary_exact_match() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("cowsay"), "").unwrap();

        let result = resolve_binary("cowsay", &bin_dir, "cowsay").unwrap();
        assert_eq!(result, "cowsay");
    }

    #[test]
    fn test_resolve_binary_single_available() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("node"), "").unwrap();

        let result = resolve_binary("nodejs", &bin_dir, "nodejs").unwrap();
        assert_eq!(result, "node");
    }

    #[test]
    fn test_resolve_binary_reverse_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("node"), "").unwrap();
        std::fs::write(bin_dir.join("corepack"), "").unwrap();
        std::fs::write(bin_dir.join("npx"), "").unwrap();

        // "nodejs" starts with "node", so "node" is the reverse prefix match
        let result = resolve_binary("nodejs", &bin_dir, "nodejs").unwrap();
        assert_eq!(result, "node");
    }

    #[test]
    fn test_resolve_binary_no_match_errors() {
        let dir = tempfile::tempdir().unwrap();
        let bin_dir = dir.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        std::fs::write(bin_dir.join("foo"), "").unwrap();
        std::fs::write(bin_dir.join("bar"), "").unwrap();

        let result = resolve_binary("baz", &bin_dir, "somepkg");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Available binaries: bar, foo"));
    }

    // ----- BinaryPreferences -----

    #[test]
    fn test_binary_prefs_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path();

        let mut prefs = BinaryPreferences::default();
        prefs.set_and_save("vi", "vim", state_dir).unwrap();

        let loaded = BinaryPreferences::load(state_dir).unwrap();
        assert_eq!(loaded.get("vi"), Some("vim"));
    }

    #[test]
    fn test_binary_prefs_empty_default() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path();

        let prefs = BinaryPreferences::load(state_dir).unwrap();
        assert_eq!(prefs.get("vi"), None);
    }

    #[test]
    fn test_binary_prefs_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path();

        let mut prefs = BinaryPreferences::default();
        prefs.set_and_save("vi", "vim", state_dir).unwrap();
        prefs.set_and_save("vi", "neovim", state_dir).unwrap();

        let loaded = BinaryPreferences::load(state_dir).unwrap();
        assert_eq!(loaded.get("vi"), Some("neovim"));
    }

    #[test]
    fn test_binary_prefs_clear() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path();

        let mut prefs = BinaryPreferences::default();
        prefs.set_and_save("vi", "vim", state_dir).unwrap();
        prefs.clear_and_save("vi", state_dir).unwrap();

        let loaded = BinaryPreferences::load(state_dir).unwrap();
        assert_eq!(loaded.get("vi"), None);
    }

    #[test]
    fn test_binary_prefs_multiple_entries() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path();

        let mut prefs = BinaryPreferences::default();
        prefs.set_and_save("vi", "vim", state_dir).unwrap();
        prefs.set_and_save("jq", "jq", state_dir).unwrap();

        let loaded = BinaryPreferences::load(state_dir).unwrap();
        assert_eq!(loaded.get("vi"), Some("vim"));
        assert_eq!(loaded.get("jq"), Some("jq"));
    }

    // ----- non_interactive_ambiguity_error -----

    #[test]
    fn test_non_interactive_error_lists_candidates() {
        let candidates = vec![
            PackageCandidate {
                attr_path: "vim".to_string(),
                pname: "vim".to_string(),
                description: None,
                version: None,
            },
            PackageCandidate {
                attr_path: "vimer".to_string(),
                pname: "vimer".to_string(),
                description: None,
                version: None,
            },
        ];
        let err = non_interactive_ambiguity_error("vi", &candidates);
        assert!(err.contains("vim"));
        assert!(err.contains("vimer"));
        assert!(err.contains("--package"));
    }
}
