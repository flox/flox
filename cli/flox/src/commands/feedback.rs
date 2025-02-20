use anyhow::bail;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use inquire::validator::{ErrorMessage, StringValidator, Validation};
use inquire::{Editor, Text};
use sentry::protocol::Feedback as UserFeedback;
use sentry::{Envelope, Hub};

use crate::config::Config;

#[derive(Debug, Clone, Bpaf)]
pub struct Feedback {
    /// The name to send the feedback under.
    #[bpaf(short, long)]
    pub name: Option<String>,
    /// A contact email in case you're ok with us getting in touch based
    /// on your feedback.
    #[bpaf(short, long)]
    pub email: Option<String>,
    /// Print the Sentry envelope instead of sending the feedback
    #[bpaf(short, long)]
    pub print: bool,
}

impl Feedback {
    pub async fn handle(&self, config: Config, _flox: Flox) -> Result<(), anyhow::Error> {
        if config.flox.disable_metrics {
            bail!("Can't send feedback without enabling metrics. Sorry!");
        }
        let mut name_prompt =
            Text::new("Name:").with_help_message("This is optional (Esc to skip)");
        if let Some(ref name) = self.name {
            name_prompt = name_prompt.with_initial_value(name.as_str());
        }
        let name = name_prompt.prompt_skippable()?;
        let mut email_prompt = Text::new("Email:")
            .with_help_message("This is optional in case you'd like us to respond (Esc to skip)");
        if let Some(ref email) = self.email {
            email_prompt = email_prompt.with_help_message(email.as_str());
        }
        let contact_email = email_prompt.prompt_skippable()?;
        let message_prompt = Editor::new("Start writing a message:")
            .with_help_message(
                "You'll brought back here after closing your editor (Enter to submit)",
            )
            .with_validator(EmptySubmissionValidator);
        let message = message_prompt.prompt()?;
        Hub::with_active(|hub| {
            if let Some(client) = hub.client() {
                let feedback = UserFeedback {
                    contact_email,
                    name,
                    message,
                };
                let envelope: Envelope = feedback.into();
                if self.print {
                    print_envelope(&envelope);
                } else {
                    client.send_envelope(envelope);
                }
            }
        });
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
    eprintln!("{output}");
}
