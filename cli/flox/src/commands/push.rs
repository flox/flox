use std::str::FromStr;

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::{EnvironmentOwner, Flox};
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironment,
    ManagedEnvironmentError,
    PushResult,
};
use flox_rust_sdk::models::environment::path_environment::PathEnvironment;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{
    ConcreteEnvironment,
    Environment,
    EnvironmentError,
    ManagedPointer,
};
use indoc::formatdoc;
use tracing::{debug, instrument};

use crate::commands::check_for_upgrades::invalidate_cached_remote_state;
use crate::commands::{EnvironmentSelect, ensure_floxhub_token, environment_select};
use crate::subcommand_metric;
use crate::utils::errors::format_core_error;
use crate::utils::message;

// Send environment to FloxHub
//
// For managed environments in a directory:
//   flox push [--dir <path>]
//
// For cached remote environments:
//   flox push -r <owner>/<name>
#[derive(Bpaf, Clone)]
pub struct Push {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// FloxHub account to push the environment to (default: current FloxHub user).
    /// Can only be specified when pushing an environment for the first time.
    /// Organizations may use either '--owner=<orgname>' or alias '--org=<orgname>'.
    #[bpaf(long("owner"), long("org"), short('o'), argument("owner"))]
    owner: Option<EnvironmentOwner>,

    /// Forcibly overwrite the remote copy of the environment
    #[bpaf(long, short)]
    force: bool,
}

impl Push {
    #[instrument(name = "push", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        subcommand_metric!("push");

        // Ensure the user is logged in for the following remote operations
        ensure_floxhub_token(&mut flox).await?;

        // Start a span that doesn't include authentication
        let span = tracing::info_span!("post-auth");
        let _guard = span.enter();

        if let EnvironmentSelect::Remote(env_ref) = &self.environment {
            let pointer = ManagedPointer::new(
                env_ref.owner().clone(),
                env_ref.name().clone(),
                &flox.floxhub,
            );

            // Check if the remote environment is cached locally
            if !RemoteEnvironment::is_cached(&flox, &pointer) {
                bail!(formatdoc! {"
                    Remote environment {env_ref} not found in local cache.

                    Have you activated or pulled this environment?
                    Try: flox activate -r {env_ref}
                "});
            }
        }

        let env = self
            .environment
            .detect_concrete_environment(&flox, "Push")?;

        match (env, self.owner) {
            (ConcreteEnvironment::Managed(managed_environment), Some(owner)) => {
                cant_change_owner_error(managed_environment.pointer(), owner)?
            },
            (ConcreteEnvironment::Remote(remote_environment), Some(owner)) => {
                cant_change_owner_error(remote_environment.pointer(), owner)?
            },
            (ConcreteEnvironment::Path(path_environment), owner) => {
                handle_path_environment_push(&flox, path_environment, owner, self.force)?
            },
            (ConcreteEnvironment::Managed(managed_environment), None) => {
                handle_managed_environment_push(&flox, managed_environment, self.force)?
            },
            (ConcreteEnvironment::Remote(remote_environment), None) => {
                handle_remote_environment_push(&flox, remote_environment, self.force)?
            },
        }

        Ok(())
    }
}

fn handle_path_environment_push(
    flox: &Flox,
    path_environment: PathEnvironment,
    owner: Option<EnvironmentOwner>,
    force: bool,
) -> Result<()> {
    let owner = if let Some(owner) = owner {
        owner
    } else {
        EnvironmentOwner::from_str(
            flox.floxhub_token
                .as_ref()
                .context("Need to be logged in")?
                .handle(),
        )?
    };

    let pointer = ManagedPointer::new(owner.clone(), path_environment.name(), &flox.floxhub);

    let managed_environment = ManagedEnvironment::push_new(flox, path_environment, owner, force)
        .map_err(|err| convert_error(err, pointer, true))?;

    message::updated(push_message(managed_environment.pointer(), force, true)?);
    Ok(())
}

fn handle_managed_environment_push(
    flox: &Flox,
    mut environment: ManagedEnvironment,
    force: bool,
) -> Result<()> {
    let pointer = environment.pointer().clone();

    let push_result = environment
        .push(flox, force)
        .map_err(|err| convert_error(err, pointer.clone(), false))?;

    // avoid false environment upgrade notifications after referring to outdated remote state
    let _ =
        invalidate_cached_remote_state(&mut environment.into()).inspect_err(|invalidation_error| {
            debug!(%invalidation_error, "failed to invalidate cached remote state");
        });

    match push_result {
        PushResult::Updated => {
            let message = push_message(&pointer, force, false)?;
            message::updated(message);
        },
        PushResult::UpToDate => {
            message::info(formatdoc! {"
                No changes to push for {name}.
                The environment on FloxHub is already up to date.
            ", name = pointer.name});
        },
    }

    Ok(())
}

/// Handle pushing a cached remote environment
fn handle_remote_environment_push(
    flox: &Flox,
    mut remote_env: RemoteEnvironment,
    force: bool,
) -> Result<()> {
    // Open the remote environment and push changes
    let push_result = remote_env.push(flox, force)?;

    match push_result {
        PushResult::Updated => {
            let message = push_message(remote_env.pointer(), force, false)?;
            message::updated(message);
        },
        PushResult::UpToDate => {
            message::info(formatdoc! {"
                No changes to push for {name}.
                The environment on FloxHub is already up to date.
            ", name = remote_env.name()});
        },
    }

    // avoid false environment upgrade notifications after referring to outdated remote state
    let _ =
        invalidate_cached_remote_state(&mut remote_env.into()).inspect_err(|invalidation_error| {
            debug!(%invalidation_error, "failed to invalidate cached remote state");
        });

    Ok(())
}

/// Construct a message for pushing an environment to FloxHub.
fn push_message(env: &ManagedPointer, force: bool, new: bool) -> Result<String> {
    let owner = &env.owner;
    let name = &env.name;
    let url = &env.floxhub_url()?;

    let force_prefix = if force { "force " } else { "" };
    let heading = if new {
        format!("{name} successfully {force_prefix}pushed to FloxHub as public")
    } else {
        format!("Updates to {name} successfully {force_prefix}pushed to FloxHub")
    };

    Ok(formatdoc! {"
        {heading}

        View the environment at: {url}
        Use this environment from another machine: 'flox activate -r {owner}/{name}'
        Make a copy of this environment: 'flox pull {owner}/{name}'
    "})
}

fn convert_error(
    err: EnvironmentError,
    pointer: ManagedPointer,
    create_remote: bool,
) -> anyhow::Error {
    let owner = &pointer.owner;
    let name = &pointer.name;

    let message = match err {
        EnvironmentError::ManagedEnvironment(ManagedEnvironmentError::AccessDenied) => {
            formatdoc! {"
            You do not have permission to write to {owner}/{name}
        "}
            .into()
        },
        EnvironmentError::ManagedEnvironment(ManagedEnvironmentError::UpstreamAlreadyExists {
            ref env_ref,
            ..
        }) if create_remote => formatdoc! {"
            An environment named {env_ref} already exists!

            To rename your environment: 'flox edit --name <new name>'
            To pull and manually re-apply your changes: 'flox delete && flox pull -r {owner}/{name}'
            To overwrite and force update: 'flox push --force'
        "}
        .into(),
        EnvironmentError::ManagedEnvironment(ManagedEnvironmentError::Build(ref err)) => {
            formatdoc! {"
            {err}

            Unable to push environment with build errors.

            Use 'flox edit' to resolve errors, test with 'flox activate', and 'flox push' again.",
                err = format_core_error(err)
            }
            .into()
        },
        _ => None,
    };

    // todo: add message to error using `context` when we work more on polishing errors
    if let Some(message) = message {
        debug!("converted error to message: {err:?} -> {message}");
        anyhow::Error::msg(message)
    } else {
        err.into()
    }
}

fn cant_change_owner_error(pointer: &ManagedPointer, owner: EnvironmentOwner) -> Result<()> {
    bail!(formatdoc! {"
        Cannot change the owner of an environment already pushed to FloxHub.

        To push this environment to another owner or org:
        * Push any outstanding changes with 'flox push'
        * Create copy of the environment with 'flox pull --copy -d <directory> {existing_owner}/{existing_name}'
        * Push the copy to the new owner with 'flox push --owner {owner}'
    ", existing_owner = pointer.owner, existing_name = pointer.name});
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use flox_rust_sdk::flox::EnvironmentOwner;
    use flox_rust_sdk::flox::test_helpers::{
        create_test_token,
        flox_instance_with_optional_floxhub,
    };
    use flox_rust_sdk::models::environment::managed_environment::test_helpers::mock_managed_environment_in;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment_in;
    use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
    use flox_rust_sdk::models::environment::{Environment, ManagedPointer};
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use tracing::instrument::WithSubscriber;

    use super::Push;
    use crate::commands::EnvironmentSelect;

    const EMPTY_MANIFEST: &str = "version = 1";

    #[tokio::test]
    async fn push_new_environment() {
        let name = "my-env";
        let owner = EnvironmentOwner::from_str("owner").unwrap();

        let (mut flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let token = create_test_token(owner.as_str());
        flox.floxhub_token = Some(token);
        let (subscriber, writer) = test_subscriber_message_only();

        let env = new_path_environment_in(&flox, EMPTY_MANIFEST, tempdir.path().join(name));
        let push_cmd = Push {
            environment: EnvironmentSelect::Dir(env.parent_path().unwrap()),
            owner: Some(owner),
            force: false,
        };

        push_cmd
            .handle(flox)
            .with_subscriber(subscriber)
            .await
            .unwrap();

        assert_eq!(writer.to_string(), indoc! {"
            ✅ my-env successfully pushed to FloxHub as public

            View the environment at: https://hub.flox.dev/owner/my-env
            Use this environment from another machine: 'flox activate -r owner/my-env'
            Make a copy of this environment: 'flox pull owner/my-env'

        "});
    }

    #[tokio::test]
    async fn push_changes() {
        let name = "my-env";
        let owner = EnvironmentOwner::from_str("owner").unwrap();

        let (mut flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let token = create_test_token(owner.as_str());
        flox.floxhub_token = Some(token);
        let (subscriber, writer) = test_subscriber_message_only();

        let mut env = mock_managed_environment_in(
            &flox,
            EMPTY_MANIFEST,
            owner.clone(),
            tempdir.path().join(name),
            Some(name),
        );
        let push_cmd = Push {
            environment: EnvironmentSelect::Dir(env.parent_path().unwrap()),
            owner: None,
            force: false,
        };

        let updated_manifest = indoc! {"
            # load bearing comment
            version = 1
        "};
        env.edit(&flox, updated_manifest.to_string()).unwrap();

        push_cmd
            .handle(flox)
            .with_subscriber(subscriber)
            .await
            .unwrap();

        assert_eq!(writer.to_string(), indoc! {"
            ✅ Updates to my-env successfully pushed to FloxHub

            View the environment at: https://hub.flox.dev/owner/my-env
            Use this environment from another machine: 'flox activate -r owner/my-env'
            Make a copy of this environment: 'flox pull owner/my-env'

        "});
    }

    #[tokio::test]
    async fn push_no_changes() {
        let name = "my-env";
        let owner = EnvironmentOwner::from_str("owner").unwrap();

        let (mut flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let token = create_test_token(owner.as_str());
        flox.floxhub_token = Some(token);
        let (subscriber, writer) = test_subscriber_message_only();

        let env = mock_managed_environment_in(
            &flox,
            EMPTY_MANIFEST,
            owner.clone(),
            tempdir.path().join(name),
            Some(name),
        );
        let push_cmd = Push {
            environment: EnvironmentSelect::Dir(env.parent_path().unwrap()),
            owner: None,
            force: false,
        };

        push_cmd
            .handle(flox)
            .with_subscriber(subscriber)
            .await
            .unwrap();

        assert_eq!(writer.to_string(), indoc! {"
            ℹ️  No changes to push for my-env.
            The environment on FloxHub is already up to date.

        "});
    }

    #[tokio::test]
    async fn push_remote_not_cached_fails() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();

        let (mut flox, _tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let token = create_test_token(owner.as_str());
        flox.floxhub_token = Some(token);

        let env_ref = format!("{}/my-env", owner).parse().unwrap();
        let push_cmd = Push {
            environment: EnvironmentSelect::Remote(env_ref),
            owner: None,
            force: false,
        };

        let result = push_cmd.handle(flox).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found in local cache"),
            "in: {err_msg}"
        );
        assert!(err_msg.contains("flox activate -r"), "in: {err_msg}");
    }

    #[tokio::test]
    async fn push_remote_changes() {
        let name = "my-env";
        let owner = EnvironmentOwner::from_str("owner").unwrap();

        let (mut flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let token = create_test_token(owner.as_str());
        flox.floxhub_token = Some(token);

        // Create and push a managed environment to mock FloxHub
        let mut env = mock_managed_environment_in(
            &flox,
            EMPTY_MANIFEST,
            owner.clone(),
            tempdir.path().join(name),
            Some(name),
        );
        // Use the internal push method directly
        env.push(&flox, false).unwrap();

        // Now open it as a remote environment (this will cache it)
        let pointer = ManagedPointer::new(owner.clone(), name.parse().unwrap(), &flox.floxhub);
        let mut remote_env = RemoteEnvironment::new(&flox, pointer, None).unwrap();

        // Make changes to the cached remote environment
        let updated_manifest = indoc! {"
            # load bearing comment
            version = 1
        "};
        remote_env
            .edit(&flox, updated_manifest.to_string())
            .unwrap();

        // Push the remote environment changes using -r
        let (subscriber, writer) = test_subscriber_message_only();
        let env_ref = remote_env.env_ref();
        let push_remote_cmd = Push {
            environment: EnvironmentSelect::Remote(env_ref),
            owner: None,
            force: false,
        };

        push_remote_cmd
            .handle(flox)
            .with_subscriber(subscriber)
            .await
            .unwrap();

        assert!(
            writer
                .to_string()
                .contains("Updates to my-env successfully pushed to FloxHub")
        );
        assert!(writer.to_string().contains("flox activate -r owner/my-env"));
    }

    #[tokio::test]
    async fn push_remote_no_changes() {
        let name = "my-env";
        let owner = EnvironmentOwner::from_str("owner").unwrap();

        let (mut flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let token = create_test_token(owner.as_str());
        flox.floxhub_token = Some(token);

        // Create and push a managed environment to mock FloxHub
        let mut env = mock_managed_environment_in(
            &flox,
            EMPTY_MANIFEST,
            owner.clone(),
            tempdir.path().join(name),
            Some(name),
        );
        // Use the internal push method directly
        env.push(&flox, false).unwrap();

        // Now open it as a remote environment (this will cache it)
        let pointer = ManagedPointer::new(owner.clone(), name.parse().unwrap(), &flox.floxhub);
        let remote_env = RemoteEnvironment::new(&flox, pointer, None).unwrap();

        // Push the remote environment without making changes using -r
        let (subscriber, writer) = test_subscriber_message_only();
        let env_ref = remote_env.env_ref();
        let push_remote_cmd = Push {
            environment: EnvironmentSelect::Remote(env_ref),
            owner: None,
            force: false,
        };

        push_remote_cmd
            .handle(flox)
            .with_subscriber(subscriber)
            .await
            .unwrap();

        assert_eq!(writer.to_string(), indoc! {"
            ℹ️  No changes to push for my-env.
            The environment on FloxHub is already up to date.

        "});
    }
}
