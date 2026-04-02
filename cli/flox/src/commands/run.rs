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

        let concrete_environment = ConcreteEnvironment::Path(path_env);

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
}
