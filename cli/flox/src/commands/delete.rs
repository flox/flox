use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_core::data::environment_ref::RemoteEnvironmentRef;
use flox_events::{CliEnvironmentPayload, EventKind, EventsHub};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment, ManagedPointer};
use indoc::formatdoc;
use tracing::{debug, instrument};

use crate::commands::{DirEnvironmentSelect, dir_environment_select, environment_description};
use crate::utils::dialog::{Confirm, Dialog};
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;
use crate::{environment_subcommand_metric, subcommand_metric};

// Delete an environment
#[derive(Bpaf, Clone)]
pub struct Delete {
    /// Delete an environment without confirmation.
    #[bpaf(short, long)]
    force: bool,

    /// Delete the local copy of a FloxHub environment.
    ///
    /// Removes the copy cached on this machine by `flox activate --reference`
    /// or `flox pull --reference`. The environment on FloxHub is not deleted.
    #[bpaf(long("reference"), long("ref"), short('r'), argument("owner>/<name"))]
    remote: Option<RemoteEnvironmentRef>,

    // TODO: switch back to `EnvironmentSelect` once we implement
    // <https://github.com/flox/flox/issues/3391>
    #[bpaf(external(dir_environment_select), fallback(Default::default()))]
    environment: DirEnvironmentSelect,
}

impl Delete {
    #[instrument(name = "delete", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        // `-r`/`--reference` deletes only the local cached copy of a FloxHub
        // environment; the upstream environment is never touched.
        if let Some(env_ref) = self.remote.clone() {
            if matches!(self.environment, DirEnvironmentSelect::Dir(_)) {
                bail!("`--reference` cannot be combined with `-d`.");
            }
            return delete_local_remote_copy(&flox, env_ref, self.force).await;
        }

        let environment = self
            .environment
            .detect_concrete_environment(&mut flox, "Delete")?;

        environment_subcommand_metric!("delete", environment);
        if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentDelete(
            CliEnvironmentPayload::new(env_detail_from_concrete(&environment)),
        )) {
            debug!(error = %err, "Failed to record v2 event");
        }

        let description = environment_description(&environment)?;

        if matches!(environment, ConcreteEnvironment::Remote(_)) {
            let message = formatdoc! {"
                Environment {description} was not deleted.

                Remote environments on FloxHub cannot yet be deleted.
            "};
            bail!("{message}")
        }

        // TODO: Inform about `--upstream` option once we implement
        // <https://github.com/flox/flox/issues/3391>
        if let ConcreteEnvironment::Managed(ref env) = environment {
            let dot_flox = env.dot_flox_path();
            let dot_flox = dot_flox.display();

            let message = formatdoc! {"
                Environment {description} is linked with a FloxHub environment.

                FloxHub environments cannot yet be deleted.
                This command will only delete the local link in '{dot_flox}'.
            "};
            message::warning(message);
        }

        let message = if let DirEnvironmentSelect::Unspecified = self.environment {
            format!("You are about to delete your environment {description}. Are you sure?")
        } else {
            "Are you sure?".to_string()
        };

        let confirm = Dialog {
            message: &message,
            help_message: Some("Use `-f` to force deletion"),
            typed: Confirm {
                default: Some(false),
            },
        };

        if !self.force && Dialog::can_prompt() && !confirm.prompt().await? {
            bail!("Environment deletion cancelled");
        }

        match environment {
            ConcreteEnvironment::Path(environment) => environment.delete(&flox),
            ConcreteEnvironment::Managed(environment) => environment.delete(&flox),
            ConcreteEnvironment::Remote(_) => unreachable!(),
        }?;

        message::deleted(format!("environment {description} deleted"));

        Ok(())
    }
}

/// Delete the local cached copy of a remote (FloxHub) environment.
///
/// The copy is the one created on this machine by `flox activate --reference`
/// or `flox pull --reference`. This does not delete the environment on FloxHub,
/// and is a local operation that doesn't require network access.
async fn delete_local_remote_copy(
    flox: &Flox,
    env_ref: RemoteEnvironmentRef,
    force: bool,
) -> Result<()> {
    subcommand_metric!("delete", remote_environment = env_ref.to_string());

    let pointer = ManagedPointer::new(
        env_ref.owner().clone(),
        env_ref.name().clone(),
        &flox.floxhub,
    );

    if !RemoteEnvironment::is_cached(flox, &pointer) {
        bail!(formatdoc! {"
            No local copy of remote environment {env_ref} was found.

            A local copy is created by 'flox activate --reference {env_ref}' or 'flox pull --reference {env_ref}'.
        "});
    }

    let confirm = Dialog {
        message: &format!(
            "You are about to delete the local copy of {env_ref}. \
             The environment on FloxHub will not be deleted. Are you sure?"
        ),
        help_message: Some("Use `-f` to force deletion"),
        typed: Confirm {
            default: Some(false),
        },
    };

    if !force && Dialog::can_prompt() && !confirm.prompt().await? {
        bail!("Environment deletion cancelled");
    }

    RemoteEnvironment::delete_local_checkout(flox, &pointer)?;

    message::deleted(format!(
        "Local copy of environment {env_ref} deleted. \
         The environment on FloxHub was not deleted."
    ));

    Ok(())
}
