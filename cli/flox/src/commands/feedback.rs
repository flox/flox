use anyhow::bail;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use indoc::indoc;
use inquire::validator::{ErrorMessage, StringValidator, Validation};
use inquire::{Editor, Text};
use sentry::protocol::Feedback as UserFeedback;
use sentry::{Envelope, Hub};

use crate::config::Config;

#[derive(Debug, Clone, Bpaf)]
pub struct Feedback {
    // Print the Sentry envelope instead of sending the feedback
    // Only used for debugging
    #[bpaf(short, long, hide)]
    pub print: bool,
}

impl Feedback {
    pub async fn handle(&self, config: Config, _flox: Flox) -> Result<(), anyhow::Error> {
        let error_msg = indoc! {"Can't send feedback in this configuration, sorry!

        Since this is an experimental command, there are some limitations:
        - Telemetry must be enabled (can be temporarily enabled via FLOX_DISABLE_METRICS=false flox feedback)
        - Flox must be installed from an official installer, not via Nix
        "};
        if config.flox.disable_metrics {
            bail!(error_msg);
        }

        // Check whether the Sentry DSN is set _before_ a user
        // goes through the trouble of entering any information
        let hub = Hub::current();
        let maybe_sentry_client = hub.client();
        if maybe_sentry_client.is_none() {
            bail!(error_msg);
        }
        let sentry_client = maybe_sentry_client.unwrap();

        let name = Text::new("Name:")
            .with_help_message("This is optional (Esc to skip)")
            .prompt_skippable()?;
        let email = Text::new("Email:")
            .with_help_message("This is optional in case you'd like us to follow up (Esc to skip)")
            .prompt_skippable()?;
        let message = Editor::new("Open an editor to write your message:")
            .with_help_message(
                "You'll brought back here after closing your editor (Enter to submit)",
            )
            .with_validator(EmptySubmissionValidator)
            .prompt()?;

        let feedback = UserFeedback {
            contact_email: email,
            name,
            message,
        };
        let envelope: Envelope = feedback.into();
        if self.print {
            // This branch is only for debugging
            print_envelope(&envelope);
        } else {
            sentry_client.send_envelope(envelope);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct EmptySubmissionValidator;

impl StringValidator for EmptySubmissionValidator {
    fn validate(
        &self,
        input: &str,
    ) -> Result<inquire::validator::Validation, inquire::CustomUserError> {
        if input.is_empty() {
            Ok(Validation::Invalid(ErrorMessage::Custom(
                "Feedback was empty".to_string(),
            )))
        } else {
            Ok(Validation::Valid)
        }
    }
}

fn print_envelope(envelope: &Envelope) {
    let mut buf = Vec::new();
    envelope.to_writer(&mut buf).unwrap();
    let output = String::from_utf8_lossy(buf.as_slice());
    eprintln!("ENVELOPE: {output}");
}
