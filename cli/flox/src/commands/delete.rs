use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_core::data::environment_ref::{DEFAULT_NAME, RemoteEnvironmentRef};
use flox_events::{CliEnvironmentPayload, EnvDetail, EventKind, EventsHub};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment, ManagedPointer};
use indoc::formatdoc;
use tracing::{debug, instrument};

use crate::commands::{
    DirEnvironmentSelect,
    EnvironmentSelect,
    EnvironmentSelectError,
    ensure_auth,
    environment_description,
    environment_select,
};
use crate::environment_subcommand_metric;
use crate::utils::dialog::{Confirm, Dialog};
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;

// Delete an environment
#[derive(Bpaf, Clone)]
pub struct Delete {
    /// Delete an environment without confirmation.
    #[bpaf(short, long)]
    force: bool,

    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl Delete {
    #[instrument(name = "delete", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        // `-r`/`--reference` and `-D`/`--default` name a FloxHub environment,
        // for which the only supported deletion is of the copy cached on this
        // machine. Resolve the reference without opening the checkout so that
        // a copy which can no longer be activated is still removable.
        if let Some(env_ref) = self.remote_reference(&mut flox).await? {
            return delete_local_remote_copy(&flox, env_ref, self.force).await;
        }

        // Deliberately narrowed to the directory variants. `EnvironmentSelect`
        // would materialize an active remote environment over the network just
        // to reject it below, where `DirEnvironmentSelect` reports it as
        // `RemoteNotSupported` without a round trip.
        let dir_select = match self.environment {
            EnvironmentSelect::Dir(ref path) => DirEnvironmentSelect::Dir(path.clone()),
            EnvironmentSelect::Unspecified => DirEnvironmentSelect::Unspecified,
            EnvironmentSelect::Remote(_) | EnvironmentSelect::Default => {
                unreachable!("handled by remote_reference above")
            },
        };

        let environment = match dir_select.detect_concrete_environment(&mut flox, "Delete") {
            // The user is standing in an activation of a FloxHub environment.
            // Deleting it upstream is not supported, but deleting the local
            // copy is, so name that rather than stopping at the refusal.
            Err(EnvironmentSelectError::RemoteNotSupported) => bail!(formatdoc! {"
                Cannot delete an active FloxHub environment.

                The environment on FloxHub cannot be deleted from the command line.
                To remove only the copy cached on this machine, run 'flox delete --reference <OWNER>/<NAME>'.
            "}),
            other => other?,
        };

        environment_subcommand_metric!("delete", environment);
        if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentDelete(
            CliEnvironmentPayload::new(env_detail_from_concrete(&flox, &environment)),
        )) {
            debug!(error = %err, "Failed to record v2 event");
        }

        let description = environment_description(&environment)?;

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

        let message = if let DirEnvironmentSelect::Unspecified = dir_select {
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

    /// The FloxHub environment named by `-r`/`--reference` or `-D`/`--default`,
    /// or [None] when the selection names a directory instead.
    ///
    /// Resolves `--default` the same way [EnvironmentSelect] does, but stops at
    /// the reference rather than going on to open the environment.
    async fn remote_reference(&self, flox: &mut Flox) -> Result<Option<RemoteEnvironmentRef>> {
        match self.environment {
            EnvironmentSelect::Remote(ref env_ref) => Ok(Some(env_ref.clone())),
            EnvironmentSelect::Default => {
                let user_handle = ensure_auth(flox).await?;
                debug!(
                    user = %user_handle,
                    "getting default environment for logged-in user"
                );
                Ok(Some(RemoteEnvironmentRef::new(user_handle, DEFAULT_NAME)?))
            },
            EnvironmentSelect::Dir(_) | EnvironmentSelect::Unspecified => Ok(None),
        }
    }
}

/// Delete the local cached copy of a remote (FloxHub) environment.
///
/// The copy is the one created on this machine by any command run with
/// `--reference`, e.g. `flox activate --reference` or `flox pull --reference`.
/// This does not delete the environment on FloxHub, and is a local operation
/// that doesn't require network access.
async fn delete_local_remote_copy(
    flox: &Flox,
    env_ref: RemoteEnvironmentRef,
    force: bool,
) -> Result<()> {
    let pointer = ManagedPointer::new(
        env_ref.owner().clone(),
        env_ref.name().clone(),
        &flox.floxhub,
    );

    if !RemoteEnvironment::local_checkout_exists(flox, &pointer) {
        bail!(formatdoc! {"
            Did not find a local copy of remote environment {env_ref}.

            A local copy is created by any command run with '--reference', such as 'flox activate --reference {env_ref}'.
        "});
    }

    if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentDelete(
        CliEnvironmentPayload::new(EnvDetail::remote(env_ref.to_string(), None)),
    )) {
        debug!(error = %err, "Failed to record v2 event");
    }

    let confirm = Dialog {
        message: &formatdoc! {"
            You are about to delete the local copy of {env_ref}.
            The environment on FloxHub will not be deleted and will be downloaded again the next time you activate or pull it.
            Are you sure?"
        },
        help_message: Some("Use '-f' to force deletion"),
        typed: Confirm {
            default: Some(false),
        },
    };

    if !force {
        // Fail closed rather than deleting unprompted: without a terminal the
        // confirmation cannot be shown, and the caller has not opted out of it.
        if !Dialog::can_prompt() {
            bail!(formatdoc! {"
                Cannot confirm deletion because this is not an interactive terminal.

                Use '--force' to delete the local copy of {env_ref} without confirmation.
            "});
        }
        if !confirm.prompt().await? {
            bail!("Environment deletion cancelled.");
        }
    }

    RemoteEnvironment::delete_local_checkout(flox, &pointer)?;

    message::deleted(formatdoc! {"
        Local copy of environment {env_ref} deleted.
        The environment on FloxHub was not deleted; it will be downloaded again the next time you activate or pull {env_ref}."
    });

    Ok(())
}
