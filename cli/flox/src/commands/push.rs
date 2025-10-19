use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::data::CanonicalPath;
use flox_rust_sdk::flox::{EnvironmentOwner, Flox};
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironment,
    ManagedEnvironmentError,
};
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
#[derive(Bpaf, Clone)]
pub struct Push {
    /// Directory to push the environment from (default: current directory)
    #[bpaf(long, short, argument("path"), complete_shell(SHELL_COMPLETION_DIR))]
    dir: Option<PathBuf>,

    /// FloxHub account to push environment to (default: current FloxHub user).
    /// Can only be specified when pushing an environment for the first time.
    /// Organizations may use either '--owner=<orgname>' or alias '--org=<orgname>'.
    #[bpaf(long("owner"), long("org"), short, argument("owner"))]
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

        let dir = match self.dir {
            Some(d) => d,
            None => std::env::current_dir().context("could not get current directory")?,
        };

        let dot_flox = DotFlox::open_in(dir)?;
        let canonical_dot_flox_path =
            CanonicalPath::new(&dot_flox.path).expect("DotFlox path was just opened");

        match dot_flox.pointer {
            // Update an existing managed environment
            EnvironmentPointer::Managed(managed_pointer) => {
                if let Some(owner) = self.owner
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
                    &flox,
                    managed_pointer.clone(),
                    &dot_flox.path,
                    self.force,
                ) {
                    Ok(_) => {
                        let message = Self::push_message(&managed_pointer, self.force, true)?;
                        message::updated(message);
                    },
                    Err(err) => {
                        // Check if this is a NothingToPush error
                        if let Some(EnvironmentError::ManagedEnvironment(
                            ManagedEnvironmentError::NothingToPush,
                        )) = err.downcast_ref::<EnvironmentError>()
                        {
                            message::info(formatdoc! {"
                                No changes to push for {name}.
                                The environment on FloxHub is already up to date.
                            ", name = managed_pointer.name});
                        } else {
                            return Err(err);
                        }
                    },
                }
            },

            // Convert a path environment to a managed environment
            EnvironmentPointer::Path(path_pointer) => {
                let owner = if let Some(owner) = self.owner {
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
                    &flox,
                    path_pointer,
                    canonical_dot_flox_path,
                    owner,
                    self.force,
                )?;

                message::updated(Self::push_message(env.pointer(), self.force, false)?);
            },
        }
        Ok(())
    }

    fn push_managed_env(
        flox: &Flox,
        managed_pointer: ManagedPointer,
        dot_flox_dir: &Path,
        force: bool,
    ) -> Result<()> {
        let mut env = ManagedEnvironment::open(flox, managed_pointer.clone(), dot_flox_dir, None)?;

        env.push(flox, force).map_err(|err| {
            Self::convert_error(
                EnvironmentError::ManagedEnvironment(err),
                managed_pointer,
                false,
            )
        })?;

        Ok(())
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
            EnvironmentError::ManagedEnvironment(ManagedEnvironmentError::NothingToPush) => formatdoc! {"
                No changes to push for {name}.
                The environment on FloxHub is already up to date.
            "}.into(),
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
