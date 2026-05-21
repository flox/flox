use std::io::{BufWriter, stdout};

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use indoc::indoc;
use shell_gen::{Shell, ShellWithPath};

use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Deactivate {
    /// Print a deactivation script to stdout instead of showing instructions
    #[bpaf(long("print-script"), hide)]
    pub print_script: bool,
}

impl Deactivate {
    pub fn handle(self, flox: Flox) -> Result<()> {
        if !flox.features.auto_activate {
            bail!(
                "'flox deactivate' requires the auto_activate feature flag. Set FLOX_FEATURES_AUTO_ACTIVATE=true."
            );
        }

        subcommand_metric!("deactivate");

        if self.print_script {
            // Detect the current shell
            let shell = detect_shell()?;

            // Generate and print the deactivation script
            let mut writer = BufWriter::new(stdout());
            flox_activations::deactivate::generate_deactivate_script(shell, &mut writer)?;

            Ok(())
        } else {
            // Interactive mode - print instructions
            message::info(indoc! {"
                To deactivate the current environment, type 'exit' to exit your shell.

                Alternatively, you can restore environment variables with:
                  eval \"$(flox deactivate --print-script)\"
            "});

            Ok(())
        }
    }
}

/// Detect the current shell from the environment
fn detect_shell() -> Result<ShellWithPath> {
    // Try FLOX_SHELL first (set during activation)
    if let Ok(shell_str) = std::env::var("FLOX_SHELL")
        && let Ok(shell) = shell_str.parse::<Shell>()
    {
        // For deactivation, we don't need the exact path, just the shell type
        let shell_path = std::path::PathBuf::from(shell.to_string());
        return Ok(match shell {
            Shell::Bash => ShellWithPath::Bash(shell_path),
            Shell::Zsh => ShellWithPath::Zsh(shell_path),
            Shell::Fish => ShellWithPath::Fish(shell_path),
            Shell::Tcsh => ShellWithPath::Tcsh(shell_path),
        });
    }

    // Fallback: try SHELL environment variable
    if let Ok(shell_path) = std::env::var("SHELL") {
        return ShellWithPath::try_from(std::path::Path::new(&shell_path))
            .map_err(|e| anyhow::anyhow!("Unsupported shell: {}", e));
    }

    anyhow::bail!("Could not detect shell. Set SHELL environment variable.");
}
