use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use indoc::{formatdoc, indoc};

use super::{activated_environments, uninitialized_environment_description};
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Exit {}

impl Exit {
    pub fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("exit");

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
