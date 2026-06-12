use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment, ManagedPointer};
use indoc::formatdoc;
use tracing::instrument;

use crate::commands::{EnvironmentSelect, environment_description, environment_select};
use crate::environment_subcommand_metric;
use crate::utils::dialog::{Confirm, Dialog};
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
        if let EnvironmentSelect::Remote(env_ref) = &self.environment {
            let pointer = ManagedPointer::new(
                env_ref.owner().clone(),
                env_ref.name().clone(),
                &flox.floxhub,
            );

            if !RemoteEnvironment::is_cached(&flox, &pointer) {
                bail!(formatdoc! {"
                    Remote environment {env_ref} not found in local cache.

                    Have you activated or pulled this environment?
                    Try: flox activate -r {env_ref}
                "});
            }
        }

        let environment = self
            .environment
            .detect_concrete_environment(&mut flox, "Delete")
            .await?;

        environment_subcommand_metric!("delete", environment);

        let description = environment_description(&environment)?;

        // TODO: Inform about `--upstream` option once we implement
        // <https://github.com/flox/flox/issues/3391>
        if let ConcreteEnvironment::Managed(ref env) = environment {
            let dot_flox = env.dot_flox_path();
            let dot_flox = dot_flox.display();

            let message = formatdoc! {"
                Environment {description} is linked with a FloxHub environment.

                FloxHub environments can not yet be deleted.
                This command will only delete the local link in '{dot_flox}'.
            "};
            message::warning(message);
        }

        let message = if let EnvironmentSelect::Unspecified = self.environment {
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
            ConcreteEnvironment::Remote(environment) => environment.delete(&flox),
        }?;

        message::deleted(format!("environment {description} deleted"));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use flox_core::data::environment_ref::EnvironmentOwner;
    use flox_rust_sdk::flox::test_helpers::{flox_instance_with_optional_floxhub, set_test_auth};
    use flox_rust_sdk::models::environment::managed_environment::test_helpers::mock_managed_environment_in;
    use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use indoc::indoc;
    use tracing::instrument::WithSubscriber;

    use super::*;
    use crate::commands::EnvironmentSelect;

    const EMPTY_MANIFEST: &str = "version = 1";

    #[tokio::test]
    async fn delete_remote_not_cached_fails() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();

        let (mut flox, _tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        set_test_auth(&mut flox, owner.as_str());

        let env_ref = format!("{owner}/my-env").parse().unwrap();
        let delete_cmd = Delete {
            force: true,
            environment: EnvironmentSelect::Remote(env_ref),
        };

        let result = delete_cmd.handle(flox).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found in local cache"),
            "in: {err_msg}"
        );
        assert!(err_msg.contains("flox activate -r"), "in: {err_msg}");
    }

    #[tokio::test]
    async fn delete_cached_remote_environment() {
        let name = "my-env";
        let owner = EnvironmentOwner::from_str("owner").unwrap();

        let (mut flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        set_test_auth(&mut flox, owner.as_str());

        let mut env = mock_managed_environment_in(
            &flox,
            EMPTY_MANIFEST,
            owner.clone(),
            tempdir.path().join(name),
            Some(name),
        );
        env.push(&flox, false).unwrap();

        let pointer = ManagedPointer::new(owner.clone(), name.parse().unwrap(), &flox.floxhub);
        let remote_env = RemoteEnvironment::new(&flox, pointer.clone(), None).unwrap();
        assert!(RemoteEnvironment::is_cached(&flox, &pointer));
        let cache_path = remote_env.parent_path().unwrap();

        let env_ref = remote_env.env_ref();
        let (subscriber, writer) = test_subscriber_message_only();
        let delete_cmd = Delete {
            force: true,
            environment: EnvironmentSelect::Remote(env_ref),
        };

        delete_cmd
            .handle(flox)
            .with_subscriber(subscriber)
            .await
            .unwrap();

        assert!(!cache_path.exists());
        assert_eq!(writer.to_string(), indoc! {"
            ━ environment 'owner/my-env' (local) deleted

        "});
    }
}
