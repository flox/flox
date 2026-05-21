use std::io::{BufWriter, stdout};

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::utils::FLOX_INTERPRETER;
use indoc::indoc;
use shell_gen::{Shell, ShellWithPath};

use crate::commands::activate::ActivateOptions;
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
            // TODO: might make sense to move detect_shell_for_in_place
            // off ActivateOptions
            let shell = ActivateOptions::detect_shell_for_in_place()?;

            // Generate and print the deactivation script
            let mut writer = BufWriter::new(stdout());
            flox_activations::deactivate::generate_deactivate_script(
                shell,
                &mut writer,
                &*FLOX_INTERPRETER,
            )?;

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
