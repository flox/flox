use std::io::{BufWriter, stdout};

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::utils::FLOX_INTERPRETER;
use indoc::{formatdoc, indoc};

use super::{activated_environments, uninitialized_environment_description};
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
            let active_environments = activated_environments();
            let last_active = active_environments.last_active();

            let Some(last_active) = last_active else {
                message::info(indoc! {"
                    No environment active!
                    Exit active environments by typing 'exit' to exit your current shell or close your terminal.
                    Environments can be activated using `flox activate`.
                "});

                return Ok(());
            };

            message::info(formatdoc! {"
                Exit the currently active environment {} by typing 'exit' to exit your current shell or close your terminal.
            ", uninitialized_environment_description(&last_active)?});

            Ok(())
        }
    }
}
