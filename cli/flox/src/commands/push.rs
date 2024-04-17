use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::{EnvironmentOwner, Flox};
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironment,
    ManagedEnvironmentError,
};
use flox_rust_sdk::models::environment::{
    path_environment,
    Environment,
    EnvironmentPointer,
    ManagedPointer,
    PathPointer,
    DOT_FLOX,
};
use indoc::formatdoc;
use log::debug;
use tracing::instrument;

use crate::commands::ensure_floxhub_token;
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::errors::format_core_error;
use crate::utils::message;

// Send environment to FloxHub
#[derive(Bpaf, Clone)]
pub struct Push {
    /// Directory to push the environment from (default: current directory)
    #[bpaf(long, short, argument("path"))]
    dir: Option<PathBuf>,

    /// Owner to push environment to (default: current user)
    #[bpaf(long, short, argument("owner"))]
    owner: Option<EnvironmentOwner>,

    /// Forceably overwrite the remote copy of the environment
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

        let dir = self.dir.unwrap_or_else(|| std::env::current_dir().unwrap());

        match EnvironmentPointer::open(&dir)? {
            EnvironmentPointer::Managed(managed_pointer) => {
                let message = Self::push_existing_message(&managed_pointer, self.force);

                Dialog {
                    message: "Pushing updates to FloxHub...",
                    help_message: None,
                    typed: Spinner::new(|| {
                        Self::push_managed_env(&flox, managed_pointer, dir, self.force)
                    }),
                }
                .spin()?;

                message::updated(message);
            },

            EnvironmentPointer::Path(path_pointer) => {
                let owner = if let Some(owner) = self.owner {
                    owner
                } else {
                    EnvironmentOwner::from_str(
                        flox.floxhub_token
                            .as_ref()
                            .context("Need to be loggedin")?
                            .handle(),
                    )?
                };

                let env = Dialog {
                    message: "Pushing environment to FloxHub...",
                    help_message: None,
                    typed: Spinner::new(|| {
                        Self::push_make_managed(&flox, path_pointer, &dir, owner, self.force)
                    }),
                }
                .spin()?;

                message::updated(Self::push_new_message(env.pointer(), self.force));
            },
        }
        Ok(())
    }

    fn push_managed_env(
        flox: &Flox,
        managed_pointer: ManagedPointer,
        dir: PathBuf,
        force: bool,
    ) -> Result<()> {
        let mut env = ManagedEnvironment::open(flox, managed_pointer.clone(), dir.join(DOT_FLOX))?;
        env.push(flox, force)
            .map_err(|err| Self::convert_error(err, managed_pointer, false))?;

        Ok(())
    }

    /// pushes a path environment in a directory to FloxHub and makes it a managed environment
    fn push_make_managed(
        flox: &Flox,
        path_pointer: PathPointer,
        dir: &Path,
        owner: EnvironmentOwner,
        force: bool,
    ) -> Result<ManagedEnvironment> {
        let dot_flox_path = dir.join(DOT_FLOX);
        let path_environment = path_environment::PathEnvironment::open(
            flox,
            path_pointer,
            dot_flox_path,
            &flox.temp_dir,
        )?;

        let pointer = ManagedPointer::new(owner.clone(), path_environment.name(), &flox.floxhub);

        let env = ManagedEnvironment::push_new(flox, path_environment, owner, force)
            .map_err(|err| Self::convert_error(err, pointer, true))?;

        Ok(env)
    }

    fn convert_error(
        err: ManagedEnvironmentError,
        pointer: ManagedPointer,
        create_remote: bool,
    ) -> anyhow::Error {
        let owner = &pointer.owner;
        let name = &pointer.name;

        let message = match err {
            ManagedEnvironmentError::AccessDenied => formatdoc! {"
                You do not have permission to write to {owner}/{name}
            "}.into(),
            ManagedEnvironmentError::Diverged if create_remote => formatdoc! {"
                An environment named {owner}/{name} already exists!

                To rename your environment: 'flox edit --name <new name>'
                To pull and manually re-apply your changes: 'flox delete && flox pull -r {owner}/{name}'
                To overwrite and force update: 'flox push --force'
            "}.into(),
            ManagedEnvironmentError::Build(ref err) => formatdoc! {"
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

    /// construct a message for an updated environment
    ///
    /// todo: add FloxHub base url when it's available
    fn push_existing_message(env: &ManagedPointer, force: bool) -> String {
        let owner = &env.owner;
        let name = &env.name;

        let suffix = if force { " (forced)" } else { "" };

        formatdoc! {"
            Updates to {name} successfully pushed to FloxHub{suffix}

            Use 'flox pull {owner}/{name}' to get this environment in any other location.
        "}
    }

    /// construct a message for a newly created environment
    ///
    /// todo: add FloxHub base url when it's available
    fn push_new_message(env: &ManagedPointer, force: bool) -> String {
        let owner = &env.owner;
        let name = &env.name;

        let suffix = if force { " (forced)" } else { "" };

        formatdoc! {"
            {name} successfully pushed to FloxHub{suffix}

            Use 'flox pull {owner}/{name}' to get this environment in any other location.
        "}
    }
}
