use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::data::CanonicalPath;
use flox_rust_sdk::flox::{EnvironmentOwner, EnvironmentRef, Flox};
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironment,
    ManagedEnvironmentError,
    PushResult,
};
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{
    DotFlox,
    Environment,
    EnvironmentError,
    EnvironmentPointer,
    ManagedPointer,
    PathPointer,
    path_environment,
};
use indoc::formatdoc;
use tracing::{debug, instrument};

use crate::commands::{SHELL_COMPLETION_DIR, ensure_floxhub_token};
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
    #[bpaf(external(push_mode))]
    mode: PushMode,

    /// Forcibly overwrite the remote copy of the environment
    #[bpaf(long, short)]
    force: bool,
}

/// Determines the mode of push operation
#[derive(Bpaf, Clone)]
enum PushMode {
    /// Push from a directory (managed or path environment)
    Directory {
        /// Directory to push the environment from (default: current directory)
        #[bpaf(
            long,
            short('d'),
            argument("path"),
            complete_shell(SHELL_COMPLETION_DIR)
        )]
        dir: Option<PathBuf>,

        /// FloxHub account to push environment to (default: current FloxHub user).
        /// Can only be specified when pushing an environment for the first time.
        /// Organizations may use either '--owner=<orgname>' or alias '--org=<orgname>'.
        #[bpaf(long("owner"), long("org"), short('o'), argument("owner"))]
        owner: Option<EnvironmentOwner>,
    },
    /// Push a cached remote environment
    Remote {
        /// Push a remote environment by reference (e.g., owner/name)
        #[bpaf(long("remote"), short('r'), argument("owner>/<name"))]
        env_ref: EnvironmentRef,
    },
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

        match &self.mode {
            PushMode::Directory { dir, owner } => {
                self.handle_directory_push(&flox, dir.clone(), owner.clone())?;
            },
            PushMode::Remote { env_ref } => {
                self.handle_remote_push(&flox, env_ref.clone())?;
            },
        }

        Ok(())
    }

    /// Handle pushing a cached remote environment
    fn handle_remote_push(&self, flox: &Flox, env_ref: EnvironmentRef) -> Result<()> {
        let pointer = ManagedPointer::new(
            env_ref.owner().clone(),
            env_ref.name().clone(),
            &flox.floxhub,
        );

        // Check if the remote environment is cached locally
        if !RemoteEnvironment::is_cached(flox, &pointer) {
            bail!(formatdoc! {"
                Remote environment {env_ref} not found in local cache.

                Have you activated or pulled this environment?
                Try: flox activate -r {env_ref}
            "});
        }

        // Open the remote environment and push changes
        let mut remote_env = RemoteEnvironment::new(flox, pointer.clone(), None)?;
        let push_result = remote_env.push(flox, self.force)?;

        match push_result {
            PushResult::Updated => {
                let message = Self::push_message(&pointer, self.force, false)?;
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

    /// Handle pushing an environment from a directory
    fn handle_directory_push(
        &self,
        flox: &Flox,
        dir: Option<PathBuf>,
        owner: Option<EnvironmentOwner>,
    ) -> Result<()> {
        let dir = match dir {
            Some(d) => d,
            None => std::env::current_dir().context("could not get current directory")?,
        };

        let dot_flox = DotFlox::open_in(dir)?;
        let canonical_dot_flox_path =
            CanonicalPath::new(&dot_flox.path).expect("DotFlox path was just opened");

        match dot_flox.pointer {
            // Update an existing managed environment
            EnvironmentPointer::Managed(managed_pointer) => {
                let new = false;
                if let Some(owner) = owner
                    && owner != managed_pointer.owner
                {
                    bail!(formatdoc! {"
                        Cannot change the owner of an environment already pushed to FloxHub.

                        To push this environment to another owner or org:
                        * Push any outstanding changes with 'flox push'
                        * Create copy of the environment with 'flox pull --copy -d <directory> {existing_owner}/{existing_name}'
                        * Push the copy to the new owner with 'flox push --owner {owner}'
                    ", existing_owner = managed_pointer.owner, existing_name = managed_pointer.name});
                }

                match Self::push_managed_env(
                    flox,
                    managed_pointer.clone(),
                    &dot_flox.path,
                    self.force,
                ) {
                    Ok(PushResult::Updated) => {
                        let message = Self::push_message(&managed_pointer, self.force, new)?;
                        message::updated(message);
                    },
                    Ok(PushResult::UpToDate) => {
                        message::info(formatdoc! {"
                            No changes to push for {name}.
                            The environment on FloxHub is already up to date.
                        ", name = managed_pointer.name});
                    },
                    Err(err) => return Err(err),
                }
            },

            // Convert a path environment to a managed environment
            EnvironmentPointer::Path(path_pointer) => {
                let new = true;
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

                let env = Self::push_make_managed(
                    flox,
                    path_pointer,
                    canonical_dot_flox_path,
                    owner,
                    self.force,
                )?;

                message::updated(Self::push_message(env.pointer(), self.force, new)?);
            },
        }
        Ok(())
    }

    fn push_managed_env(
        flox: &Flox,
        managed_pointer: ManagedPointer,
        dot_flox_dir: &Path,
        force: bool,
    ) -> Result<PushResult> {
        let mut env = ManagedEnvironment::open(flox, managed_pointer.clone(), dot_flox_dir, None)?;

        let push_result = env.push(flox, force).map_err(|err| {
            Self::convert_error(
                EnvironmentError::ManagedEnvironment(err),
                managed_pointer,
                false,
            )
        })?;

        Ok(push_result)
    }

    /// pushes a path environment in a directory to FloxHub and makes it a managed environment
    fn push_make_managed(
        flox: &Flox,
        path_pointer: PathPointer,
        dot_flox_path: CanonicalPath,
        owner: EnvironmentOwner,
        force: bool,
    ) -> Result<ManagedEnvironment> {
        let path_environment =
            path_environment::PathEnvironment::open(flox, path_pointer, dot_flox_path)?;

        let pointer = ManagedPointer::new(owner.clone(), path_environment.name(), &flox.floxhub);

        let env = ManagedEnvironment::push_new(flox, path_environment, owner, force)
            .map_err(|err| Self::convert_error(err, pointer, true))?;

        Ok(env)
    }

    fn convert_error(
        err: EnvironmentError,
        pointer: ManagedPointer,
        create_remote: bool,
    ) -> anyhow::Error {
        let owner = &pointer.owner;
        let name = &pointer.name;

        let message = match err {
            EnvironmentError::ManagedEnvironment(ManagedEnvironmentError::AccessDenied) => formatdoc! {"
                You do not have permission to write to {owner}/{name}
            "}.into(),
            EnvironmentError::ManagedEnvironment(ManagedEnvironmentError::UpstreamAlreadyExists { ref env_ref, .. }) if create_remote => formatdoc! {"
                An environment named {env_ref} already exists!

                To rename your environment: 'flox edit --name <new name>'
                To pull and manually re-apply your changes: 'flox delete && flox pull -r {owner}/{name}'
                To overwrite and force update: 'flox push --force'
            "}.into(),
            EnvironmentError::ManagedEnvironment(ManagedEnvironmentError::Build(ref err)) => formatdoc! {"
                {err}

                Unable to push environment with build errors.

                Use 'flox edit' to resolve errors, test with 'flox activate', and 'flox push' again.",
                err = format_core_error(err)
            }.into(),
            _ => None
        };

        // todo: add message to error using `context` when we work more on polishing errors
        if let Some(message) = message {
            debug!("converted error to message: {err:?} -> {message}");
            anyhow::Error::msg(message)
        } else {
            err.into()
        }
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
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use flox_rust_sdk::flox::EnvironmentOwner;
    use flox_rust_sdk::flox::test_helpers::{
        create_test_token,
        flox_instance_with_optional_floxhub,
    };
    use flox_rust_sdk::models::environment::Environment;
    use flox_rust_sdk::models::environment::managed_environment::test_helpers::mock_managed_environment_in;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment_in;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use tracing::instrument::WithSubscriber;

    use super::{Push, PushMode};

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
            mode: PushMode::Directory {
                dir: Some(env.parent_path().unwrap()),
                owner: Some(owner),
            },
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
            mode: PushMode::Directory {
                dir: Some(env.parent_path().unwrap()),
                owner: Some(owner),
            },
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
            mode: PushMode::Directory {
                dir: Some(env.parent_path().unwrap()),
                owner: Some(owner),
            },
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
            mode: PushMode::Remote { env_ref },
            force: false,
        };

        let result = push_cmd.handle(flox).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found in local cache"));
        assert!(err_msg.contains("flox activate -r"));
    }
}
