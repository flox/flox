use std::str::FromStr;

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_core::data::environment_ref::EnvironmentName;
use flox_manifest::raw::PackageToInstall;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::path_environment::PathEnvironment;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment, PathPointer};
use tracing::debug;

use super::EnvironmentSelect;
use super::activate::{Activate, CommandSelect};
use crate::config::Config;
use crate::subcommand_metric;

/// Run a command from a package without installing it into an environment.
///
/// Creates a temporary environment, installs the specified package,
/// and executes a command from it. The environment is cleaned up after
/// the command exits.
#[derive(Bpaf, Clone)]
pub struct Run {
    /// Name of the binary to execute (defaults to package name)
    #[bpaf(long("bin"), short('b'), argument("BINARY"))]
    pub bin: Option<String>,

    /// The package to run (e.g. "cowsay", "python@3.11")
    #[bpaf(positional("package"))]
    pub package: String,

    /// Arguments passed to the binary (after --)
    #[bpaf(positional("args"), strict, many)]
    pub args: Vec<String>,
}

impl Run {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("run");

        let package = PackageToInstall::parse(&flox.system, &self.package)
            .context("Failed to parse package specification")?;

        // Only support catalog packages for now
        if !matches!(package, PackageToInstall::Catalog(_)) {
            bail!(
                "flox run currently only supports catalog packages.\n\
                 Flake references and store paths are not supported."
            );
        }

        let binary_name = self
            .bin
            .clone()
            .unwrap_or_else(|| derive_binary_name(&self.package));

        debug!(
            package = %self.package,
            binary = %binary_name,
            "Creating temporary environment for flox run"
        );

        // Create a temp directory for the ephemeral environment.
        // This lives under flox.temp_dir which is a per-process TempDir
        // managed by FloxArgs::handle.
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
            self.package
        );

        path_env
            .install(&[package], &flox)
            .with_context(|| format!("Failed to install package '{}'", self.package))?;

        let mut concrete_environment = ConcreteEnvironment::Path(path_env);

        // Resolve the binary to run. If --bin was explicitly provided, use it as-is.
        // Otherwise, try to find the best binary in the built environment.
        let rendered = concrete_environment
            .rendered_env_links(&flox)
            .context("Failed to get environment paths")?;
        let bin_dir = rendered.runtime.join("bin");
        let binary_name = if self.bin.is_some() {
            // User explicitly chose a binary — validate it exists
            if bin_dir.is_dir() && !bin_dir.join(&binary_name).exists() {
                let available = list_binaries(&bin_dir);
                let list = if available.is_empty() {
                    String::new()
                } else {
                    format!("\n\nAvailable binaries: {}", available.join(", "))
                };
                bail!(
                    "Binary '{binary_name}' not found in package '{package}'.{list}",
                    package = self.package,
                );
            }
            binary_name
        } else {
            resolve_binary(&binary_name, &bin_dir, &self.package)?
        };

        // Build the exec command: [binary_name, args...]
        let mut exec_args = vec![binary_name.clone()];
        exec_args.extend(self.args.clone());

        // Reuse the Activate flow — same pattern as services start
        // (cli/flox/src/commands/services/mod.rs:402-424)
        Activate {
            environment: EnvironmentSelect::Dir(run_temp_dir),
            trust: false,
            print_script: false,
            start_services: false,
            mode: None,
            generation: None,
            // this isn't actually used because we pass invocation_type below
            command: Some(CommandSelect::ExecCommand {
                command: binary_name,
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

/// Derive a binary name from a package specification string.
///
/// Examples:
/// - "cowsay" -> "cowsay"
/// - "python@3.11" -> "python"
/// - "python3Packages.numpy" -> "numpy"
/// - "curl^bin,man" -> "curl"
fn derive_binary_name(package_spec: &str) -> String {
    package_spec
        .split('@')
        .next()
        .unwrap() // strip version
        .split('^')
        .next()
        .unwrap() // strip outputs
        .rsplit('.')
        .next()
        .unwrap() // last segment of attr path
        .to_string()
}

/// List binary names available in a bin directory.
fn list_binaries(bin_dir: &std::path::Path) -> Vec<String> {
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
/// 1. If the derived name exists in bin/, use it (e.g. "cowsay" → "cowsay")
/// 2. If only one binary exists, use it
/// 3. If the derived name is a prefix of exactly one binary, use it (e.g. "python3" → "python3.11")
/// 4. If a binary is a prefix of the derived name, use it (e.g. "nodejs" → "node")
/// 5. Otherwise, error with the list of available binaries
fn resolve_binary(
    derived_name: &str,
    bin_dir: &std::path::Path,
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
         Try: flox run --bin <BINARY> {package_spec} -- ...\n\n\
         Available binaries: {}",
        available.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
