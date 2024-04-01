use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::{EnvironmentOwner, EnvironmentRef, Flox};
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironment,
    ManagedEnvironmentError,
    PullResult,
};
use flox_rust_sdk::models::environment::path_environment::{self};
use flox_rust_sdk::models::environment::{
    CoreEnvironmentError,
    Environment,
    EnvironmentError2,
    EnvironmentPointer,
    ManagedPointer,
    PathPointer,
    UpdateResult,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
};
use flox_rust_sdk::models::lockfile::{
    Input,
    LockedManifest,
    LockedManifestError,
    TypedLockedManifest,
};
use flox_rust_sdk::models::manifest::{self};
use flox_rust_sdk::models::pkgdb::{self, ScrapeError};
use indoc::formatdoc;
use itertools::Itertools;
use log::debug;
use toml_edit::Document;
use tracing::instrument;
use url::Url;

use super::{environment_select, EnvironmentSelect};
use crate::commands::{
    ensure_floxhub_token,
    environment_description,
    open_path,
    ConcreteEnvironment,
    EnvironmentSelectError,
};
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Select, Spinner};
use crate::utils::errors::{display_chain, format_core_error, format_locked_manifest_error};
use crate::utils::message;

// Uninstall installed packages from an environment
#[derive(Bpaf, Clone)]
pub struct Uninstall {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// The install IDs of the packages to remove
    #[bpaf(positional("packages"), some("Must specify at least one package"))]
    packages: Vec<String>,
}

impl Uninstall {
    #[instrument(name = "uninstall", fields(packages), skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        subcommand_metric!("uninstall");

        // Vec<T> doesn't implement tracing::Value, so you have to join the strings
        // yourself.
        tracing::Span::current().record("packages", self.packages.iter().join(","));

        debug!(
            "uninstalling packages [{}] from {:?}",
            self.packages.as_slice().join(", "),
            self.environment
        );
        let concrete_environment = match self
            .environment
            .detect_concrete_environment(&flox, "Uninstall from")
        {
            Ok(concrete_environment) => concrete_environment,
            Err(EnvironmentSelectError::Environment(
                ref e @ EnvironmentError2::DotFloxNotFound(ref dir),
            )) => {
                bail!(formatdoc! {"
                {e}

                Create an environment with 'flox init --dir {}'", dir.to_string_lossy()
                })
            },
            Err(e @ EnvironmentSelectError::EnvNotFoundInCurrentDirectory) => {
                bail!(formatdoc! {"
                {e}

                Create an environment with 'flox init' or uninstall packages from an environment found elsewhere with 'flox uninstall {} --dir <path>'",
                self.packages.join(" ")})
            },
            Err(e) => Err(e)?,
        };

        // Ensure the user is logged in for the following remote operations
        if let ConcreteEnvironment::Remote(_) = concrete_environment {
            ensure_floxhub_token(&mut flox).await?;
        };

        let description = environment_description(&concrete_environment)?;
        let mut environment = concrete_environment.into_dyn_environment();

        let _ = Dialog {
            message: &format!("Uninstalling packages from environment {description}..."),
            help_message: None,
            typed: Spinner::new(|| environment.uninstall(self.packages.clone(), &flox)),
        }
        .spin()?;

        // Note, you need two spaces between this emoji and the package name
        // otherwise they appear right next to each other.
        self.packages.iter().for_each(|p| {
            message::deleted(format!("'{p}' uninstalled from environment {description}"))
        });
        Ok(())
    }
}

// Send environment to FloxHub
#[derive(Bpaf, Clone)]
pub struct Push {
    /// Directory to push the environment from (default: current directory)
    #[bpaf(long, short, argument("path"))]
    dir: Option<PathBuf>,

    /// Owner to push push environment to (default: current user)
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
        let path_environment =
            path_environment::PathEnvironment::open(path_pointer, dot_flox_path, &flox.temp_dir)?;

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

#[derive(Debug, Clone, Bpaf)]
enum PullSelect {
    New {
        /// ID of the environment to pull
        #[bpaf(long, short, argument("owner>/<name"))]
        remote: EnvironmentRef,
    },
    NewAbbreviated {
        /// ID of the environment to pull
        #[bpaf(positional("owner>/<name"))]
        remote: EnvironmentRef,
    },
    Existing {},
}

impl Default for PullSelect {
    fn default() -> Self {
        PullSelect::Existing {}
    }
}

// Pull environment from FloxHub
#[derive(Bpaf, Clone)]
pub struct Pull {
    /// Directory in which to create a managed environment, or directory that already contains a managed environment (default: current directory)
    #[bpaf(long, short, argument("path"))]
    dir: Option<PathBuf>,

    /// Forceably pull the environment
    /// When pulling a new environment, adds the system to the manifest if the lockfile is incompatible
    /// and ignores eval and build errors.
    /// When pulling an existing environment, overrides local changes.
    #[bpaf(long, short)]
    force: bool,

    #[bpaf(external(pull_select), fallback(Default::default()))]
    pull_select: PullSelect,
}

/// Functions that are used to prompt the user in handle_pull_result
///
/// These are passed to allow testing without prompting
struct QueryFunctions {
    query_add_system: fn(&str) -> Result<bool>,
    query_ignore_build_errors: fn() -> Result<bool>,
}

impl Pull {
    #[instrument(name = "pull", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("pull");

        match self.pull_select {
            PullSelect::New { remote } | PullSelect::NewAbbreviated { remote } => {
                let (start, complete) =
                    Self::pull_new_messages(self.dir.as_deref(), &remote, flox.floxhub.base_url());

                let dir = self.dir.unwrap_or_else(|| std::env::current_dir().unwrap());

                debug!("Resolved user intent: pull {remote:?} into {dir:?}");

                Self::pull_new_environment(&flox, dir.join(DOT_FLOX), remote, self.force, &start)?;

                message::created(complete);
            },
            PullSelect::Existing {} => {
                let dir = self.dir.unwrap_or_else(|| std::env::current_dir().unwrap());

                debug!("Resolved user intent: pull changes for environment found in {dir:?}");

                let pointer = {
                    let p = EnvironmentPointer::open(&dir)?;
                    match p {
                        EnvironmentPointer::Managed(managed_pointer) => managed_pointer,
                        EnvironmentPointer::Path(_) => bail!("Cannot pull into a path environment"),
                    }
                };

                let start_message = format!(
                    "⬇️  Remote: pulling and building {owner}/{name} from {floxhub_host}",
                    owner = pointer.owner,
                    name = pointer.name,
                    floxhub_host = flox.floxhub.base_url()
                );

                let result = Dialog {
                    message: &start_message,
                    help_message: None,
                    typed: Spinner::new(|| {
                        Self::pull_existing_environment(
                            &flox,
                            dir.join(DOT_FLOX),
                            pointer.clone(),
                            self.force,
                        )
                    }),
                }
                .spin()?;

                match result {
                    PullResult::Updated => {
                        message::updated(formatdoc! {"
                            Pulled {owner}/{name} from {floxhub_host}{suffix}

                            You can activate this environment with 'flox activate'
                            ",
                            owner = pointer.owner, name = pointer.name,
                            floxhub_host = flox.floxhub.base_url(),
                            suffix = if self.force { " (forced)" } else { "" }
                        });
                    },
                    PullResult::UpToDate => {
                        message::warning(formatdoc! {"
                            {owner}/{name} is already up to date.
                        ", owner = pointer.owner, name = pointer.name});
                    },
                }
            },
        }

        Ok(())
    }

    /// Update an existing environment with the latest version from FloxHub
    ///
    /// Opens the environment and calls [ManagedEnvironment::pull] on it,
    /// which will update the lockfile.
    fn pull_existing_environment(
        flox: &Flox,
        dot_flox_path: PathBuf,
        pointer: ManagedPointer,
        force: bool,
    ) -> Result<PullResult, EnvironmentError2> {
        let mut env = ManagedEnvironment::open(flox, pointer, dot_flox_path)?;
        let state = env.pull(force)?;
        // only build if the environment was updated
        if let PullResult::Updated = state {
            env.build(flox)?;
        }
        Ok(state)
    }

    /// Pull a new environment from FloxHub into the given directory
    ///
    /// This will create a new environment in the given directory.
    /// Uses [ManagedEnvironment::open] which will try to clone the environment.
    ///
    /// If the directory already exists, this will fail early.
    /// If opening the environment fails, the .flox/ directory will be cleaned up.
    fn pull_new_environment(
        flox: &Flox,
        dot_flox_path: PathBuf,
        env_ref: EnvironmentRef,
        force: bool,
        message: &str,
    ) -> Result<()> {
        if dot_flox_path.exists() {
            if force {
                match open_path(flox, &dot_flox_path) {
                    Ok(concrete_env) => match concrete_env {
                        ConcreteEnvironment::Path(env) => {
                            env.delete(flox)
                                .context("Failed to delete existing environment")?;
                        },
                        ConcreteEnvironment::Managed(env) => {
                            env.delete(flox)
                                .context("Failed to delete existing environment")?;
                        },
                        ConcreteEnvironment::Remote(_) => {},
                    },
                    Err(_) => {
                        fs::remove_dir_all(&dot_flox_path).context(format!(
                            "Failed to remove existing .flox directory at {:?}",
                            dot_flox_path
                        ))?;
                    },
                }
            } else {
                bail!(
                    "An environment already exists at {:?}. Use --force to overwrite.",
                    dot_flox_path
                );
            }
        }

        // region: write pointer
        let pointer = ManagedPointer::new(
            env_ref.owner().clone(),
            env_ref.name().clone(),
            &flox.floxhub,
        );
        let pointer_content =
            serde_json::to_string_pretty(&pointer).context("Could not serialize pointer")?;

        fs::create_dir_all(&dot_flox_path).context("Could not create .flox/ directory")?;
        let pointer_path = dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME);
        fs::write(pointer_path, pointer_content).context("Could not write pointer")?;

        let mut env = {
            let result = Dialog {
                message,
                help_message: None,
                typed: Spinner::new(|| ManagedEnvironment::open(flox, pointer, &dot_flox_path)),
            }
            .spin()
            .map_err(|err| Self::handle_error(flox, err));

            match result {
                Err(err) => {
                    fs::remove_dir_all(&dot_flox_path)
                        .context("Could not clean up .flox/ directory")?;
                    Err(err)?
                },
                Ok(env) => env,
            }
        };
        // endregion

        let result = Dialog {
            message,
            help_message: None,
            typed: Spinner::new(|| env.build(flox)),
        }
        .spin();

        Self::handle_pull_result(
            flox,
            result,
            &dot_flox_path,
            force,
            env,
            Dialog::can_prompt().then_some(QueryFunctions {
                query_add_system: Self::query_add_system,
                query_ignore_build_errors: Self::query_ignore_build_errors,
            }),
        )
    }

    /// Helper function for [Self::pull_new_environment] that can be unit tested.
    ///
    /// A value of [None] for query_functions represents when the user cannot be prompted.
    /// [Some] represents when the user should be prompted with the provided functions.
    fn handle_pull_result(
        flox: &Flox,
        result: Result<(), EnvironmentError2>,
        dot_flox_path: &PathBuf,
        force: bool,
        mut env: ManagedEnvironment,
        query_functions: Option<QueryFunctions>,
    ) -> Result<()> {
        match result {
            Ok(_) => {},
            Err(EnvironmentError2::Core(e)) if e.is_incompatible_system_error() => {
                let hint = formatdoc! {"
                    Use 'flox pull --force' to add your system to the manifest.
                    For more on managing systems for your environment, visit the documentation:
                    https://flox.dev/docs/tutorials/multi-arch-environments
                "};
                if !force && query_functions.is_none() {
                    fs::remove_dir_all(dot_flox_path)
                        .context("Could not clean up .flox/ directory")?;
                    bail!("{}", formatdoc! {"
                            This environment is not yet compatible with your system ({system}).

                            {hint}"
                    , system = flox.system});
                }

                // Will return OK if the user chose to abort the pull.
                // The unwrap() is only reached if !force,
                // and we return above if !force and query_functions.is_none()
                let force = force || (query_functions.unwrap().query_add_system)(&flox.system)?;
                if !force {
                    // prompt available, user chose to abort
                    fs::remove_dir_all(dot_flox_path)
                        .context("Could not clean up .flox/ directory")?;
                    bail!(formatdoc! {"
                        Did not pull the environment.

                        {hint}
                    "});
                }

                let doc = Self::amend_current_system(&env, flox)?;
                if let Err(broken_error) = env.edit_unsafe(flox, doc.to_string())? {
                    message::warning(formatdoc! {"
                        {err:#}

                        Could not build modified environment, build errors need to be resolved manually.",
                        err = anyhow!(broken_error)
                    });
                };
            },
            Err(EnvironmentError2::Core(
                ref core_err @ CoreEnvironmentError::LockedManifest(
                    ref builder_error @ LockedManifestError::BuildEnv(_),
                ),
            )) if core_err.is_incompatible_package_error() => {
                debug!(
                    "environment contains package incompatible with the current system: {err}",
                    err = display_chain(core_err)
                );

                let pkgdb_error = format_locked_manifest_error(builder_error);

                if !force && query_functions.is_none() {
                    fs::remove_dir_all(dot_flox_path)
                        .context("Could not clean up .flox/ directory")?;
                    bail!("{pkgdb_error}");
                }

                message::error(pkgdb_error);

                // The unwrap() is only reached if !force,
                // and we return above if !force and query_functions.is_none()
                if force || (query_functions.unwrap().query_ignore_build_errors)()? {
                    message::warning("Ignoring build errors and pulling the environment anyway.");
                } else {
                    fs::remove_dir_all(dot_flox_path)
                        .context("Could not clean up .flox/ directory")?;
                    bail!("Did not pull the environment.");
                }
            },
            Err(e) => {
                fs::remove_dir_all(dot_flox_path).context("Could not clean up .flox/ directory")?;
                bail!(e)
            },
        };
        Ok(())
    }

    /// construct a message for pulling a new environment
    fn pull_new_messages(
        dir: Option<&Path>,
        env_ref: &EnvironmentRef,
        floxhub_host: &Url,
    ) -> (String, String) {
        let mut start_message =
            format!("⬇️  Remote: pulling and building {env_ref} from {floxhub_host}");
        if let Some(dir) = dir {
            start_message += &format!(" into {dir}", dir = dir.display());
        } else {
            start_message += " into the current directory";
        };

        let complete_message = formatdoc! {"
            Pulled {env_ref} from {floxhub_host}

            You can activate this environment with 'flox activate'
        "};

        (start_message, complete_message)
    }

    /// if possible, prompt the user to automatically add their system to the manifest
    ///
    /// returns [Ok(None)]` if the user can't be prompted
    /// returns `[Ok(bool)]` depending on the users choice
    /// returns `[Err]` if the prompt failed or was cancelled
    fn query_add_system(system: &str) -> Result<bool> {
        let message = format!(
            "The environment you are trying to pull is not yet compatible with your system ({system})."
        );

        let help = "Use 'flox pull --force' to automatically add your system to the list of compatible systems";

        let reject_choice = "Don't pull this environment.";
        let confirm_choice = format!(
            "Pull this environment anyway and add '{system}' to the supported systems list."
        );

        let dialog = Dialog {
            message: &message,
            help_message: Some(help),
            typed: Select {
                options: [reject_choice, &confirm_choice].to_vec(),
            },
        };

        let (choice, _) = dialog.raw_prompt()?;

        Ok(choice == 1)
    }

    /// add the current system to the manifest of the given environment
    fn amend_current_system(
        env: &ManagedEnvironment,
        flox: &Flox,
    ) -> Result<Document, anyhow::Error> {
        manifest::add_system(&env.manifest_content(flox)?, &flox.system)
            .context("Could not add system to manifest")
    }

    /// Ask the user if they want to ignore build errors and pull a broken environment
    fn query_ignore_build_errors() -> Result<bool> {
        if !Dialog::can_prompt() {
            return Ok(false);
        }

        let message = "The environment you are trying to pull could not be built locally.";
        let help_message = Some("Use 'flox pull --force' to pull the environment anyway.");

        let reject_choice = "Don't pull this environment.";
        let confirm_choice = "Pull this environment anyway, 'flox edit' to address issues.";

        let dialog = Dialog {
            message,
            help_message,
            typed: Select {
                options: [reject_choice, confirm_choice].to_vec(),
            },
        };

        let (choice, _) = dialog.raw_prompt()?;

        Ok(choice == 1)
    }

    fn handle_error(flox: &Flox, err: ManagedEnvironmentError) -> anyhow::Error {
        match err {
            ManagedEnvironmentError::AccessDenied => {
                let message = "You do not have permission to pull this environment";
                anyhow::Error::msg(message)
            },
            ManagedEnvironmentError::Diverged => {
                let message = "The environment has diverged from the remote version";
                anyhow::Error::msg(message)
            },
            ManagedEnvironmentError::UpstreamNotFound(env_ref, _) => {
                let by_current_user = flox
                    .floxhub_token
                    .as_ref()
                    .map(|token| token.handle() == env_ref.owner().as_str())
                    .unwrap_or_default();
                let message = format!("The environment {env_ref} does not exist.");
                if by_current_user {
                    anyhow!(formatdoc! {"
                        {message}

                        Double check the name or create it with:

                            $ flox init --name {name}
                            $ flox push
                    ", name = env_ref.name()})
                } else {
                    anyhow!(message)
                }
            },
            _ => err.into(),
        }
    }
}

#[derive(Debug, Bpaf, Clone)]
pub enum EnvironmentOrGlobalSelect {
    /// Update the global base catalog
    #[bpaf(long("global"))]
    Global,
    Environment(#[bpaf(external(environment_select))] EnvironmentSelect),
}

impl Default for EnvironmentOrGlobalSelect {
    fn default() -> Self {
        EnvironmentOrGlobalSelect::Environment(Default::default())
    }
}

// Update the global base catalog or an environment's base catalog
#[derive(Bpaf, Clone)]
pub struct Update {
    #[bpaf(external(environment_or_global_select), fallback(Default::default()))]
    environment_or_global: EnvironmentOrGlobalSelect,

    #[bpaf(positional("inputs"), hide)]
    inputs: Vec<String>,
}
impl Update {
    #[instrument(name = "update", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("update");

        let (old_lockfile, new_lockfile, global, description) = match self.environment_or_global {
            EnvironmentOrGlobalSelect::Environment(ref environment_select) => {
                let span = tracing::info_span!("update_local");
                let _guard = span.enter();

                let concrete_environment =
                    environment_select.detect_concrete_environment(&flox, "Update")?;

                let description = Some(environment_description(&concrete_environment)?);
                let UpdateResult {
                    new_lockfile,
                    old_lockfile,
                    ..
                } = Dialog {
                    message: "Updating environment...",
                    help_message: None,
                    typed: Spinner::new(|| self.update_manifest(flox, concrete_environment)),
                }
                .spin()?;

                (
                    old_lockfile
                        .map(TypedLockedManifest::try_from)
                        .transpose()?,
                    TypedLockedManifest::try_from(new_lockfile)?,
                    false,
                    description,
                )
            },
            EnvironmentOrGlobalSelect::Global => {
                let span = tracing::info_span!("update_global");
                let _guard = span.enter();

                let UpdateResult {
                    new_lockfile,
                    old_lockfile,
                    ..
                } = Dialog {
                    message: "Updating global-manifest...",
                    help_message: None,
                    typed: Spinner::new(|| {
                        LockedManifest::update_global_manifest(&flox, self.inputs)
                    }),
                }
                .spin()?;

                (
                    old_lockfile
                        .map(TypedLockedManifest::try_from)
                        .transpose()?,
                    TypedLockedManifest::try_from(new_lockfile)?,
                    true,
                    None,
                )
            },
        };

        if let Some(ref old_lockfile) = old_lockfile {
            if new_lockfile.registry().inputs == old_lockfile.registry().inputs {
                if global {
                    message::plain("ℹ️  All global inputs are up-to-date.");
                } else {
                    message::plain(format!(
                        "ℹ️  All inputs are up-to-date in environment {}.",
                        description.as_ref().unwrap()
                    ));
                }

                return Ok(());
            }
        }

        let mut inputs_to_scrape: Vec<&Input> = vec![];

        for (input_name, new_input) in &new_lockfile.registry().inputs {
            let old_input = old_lockfile
                .as_ref()
                .and_then(|old| old.registry().inputs.get(input_name));
            match old_input {
                // unchanged input
                Some(old_input) if old_input == new_input => continue, // dont need to scrape
                // updated input
                Some(_) if global => {
                    message::plain(format!("⬆️  Updated global input '{}'.", input_name))
                },
                Some(_) => message::plain(format!(
                    "⬆️  Updated input '{}' in environment {}.",
                    input_name,
                    description.as_ref().unwrap()
                )),
                // new input
                None if global => {
                    message::plain(format!("🔒️  Locked global input '{}'.", input_name))
                },
                None => message::plain(format!(
                    "🔒️  Locked input '{}' in environment {}.",
                    input_name,
                    description.as_ref().unwrap(),
                )),
            }
            inputs_to_scrape.push(new_input);
        }

        if let Some(old_lockfile) = old_lockfile {
            for input_name in old_lockfile.registry().inputs.keys() {
                if !new_lockfile.registry().inputs.contains_key(input_name) {
                    if global {
                        message::deleted(format!(
                            "Removed unused input '{}' from global lockfile.",
                            input_name
                        ));
                    } else {
                        message::deleted(format!(
                            "Removed unused input '{}' from lockfile for environment {}.",
                            input_name,
                            description.as_ref().unwrap()
                        ));
                    }
                }
            }
        }

        if inputs_to_scrape.is_empty() {
            return Ok(());
        }

        // TODO: make this async when scraping multiple inputs
        let span = tracing::info_span!("scrape");
        let _guard = span.enter();
        let results: Vec<Result<(), ScrapeError>> = Dialog {
            message: "Generating databases for updated inputs...",
            help_message: (inputs_to_scrape.len() > 1).then_some("This may take a while."),
            typed: Spinner::new(|| {
                // TODO: rayon::par_iter
                inputs_to_scrape
                    .iter()
                    .map(|input| pkgdb::scrape_input(&input.from))
                    .collect()
            }),
        }
        .spin();
        drop(_guard);

        for result in results {
            result?;
        }

        Ok(())
    }

    fn update_manifest(
        &self,
        flox: Flox,
        concrete_environment: ConcreteEnvironment,
    ) -> Result<UpdateResult> {
        let mut environment = concrete_environment.into_dyn_environment();

        Ok(environment.update(&flox, self.inputs.clone())?)
        // .context("updating environment failed")
    }
}

// Upgrade packages in an environment
#[derive(Bpaf, Clone)]
pub struct Upgrade {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// ID of a package or pkg-group name to upgrade
    #[bpaf(positional("package or pkg-group"))]
    groups_or_iids: Vec<String>,
}
impl Upgrade {
    #[instrument(name = "upgrade", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("upgrade");

        let concrete_environment = self
            .environment
            .detect_concrete_environment(&flox, "Upgrade")?;

        let description = environment_description(&concrete_environment)?;

        let mut environment = concrete_environment.into_dyn_environment();

        let result = Dialog {
            message: "Upgrading packages...",
            help_message: None,
            typed: Spinner::new(|| environment.upgrade(&flox, &self.groups_or_iids)),
        }
        .spin()?;

        let upgraded = result.packages;

        if upgraded.is_empty() {
            if self.groups_or_iids.is_empty() {
                message::plain(format!(
                    "ℹ️  No packages need to be upgraded in environment {description}."
                ));
            } else {
                message::plain(format!(
                    "ℹ️  The specified packages do not need to be upgraded in environment {description}."
                 ) );
            }
        } else {
            for package in upgraded {
                message::plain(format!(
                    "⬆️  Upgraded '{package}' in environment {description}."
                ));
            }
        }

        Ok(())
    }
}

// Containerize an environment
#[derive(Bpaf, Clone, Debug)]
pub struct Containerize {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Path to write the container to (pass '-' to write to stdout)
    #[bpaf(short, long, argument("path"))]
    output: Option<PathBuf>,
}
impl Containerize {
    #[instrument(name = "containerize", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("containerize");

        let mut env = self
            .environment
            .detect_concrete_environment(&flox, "Upgrade")?
            .into_dyn_environment();

        let output_path = match self.output {
            Some(output) => output,
            None => std::env::current_dir()
                .context("Could not get current directory")?
                .join(format!("{}-container.tar.gz", env.name())),
        };

        let (output, output_name): (Box<dyn Write + Send>, String) =
            if output_path == Path::new("-") {
                debug!("output=stdout");

                (Box::new(std::io::stdout()), "stdout".to_string())
            } else {
                debug!("output={}", output_path.display());

                let file = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&output_path)
                    .context("Could not open output file")?;

                (Box::new(file), output_path.display().to_string())
            };

        let builder = Dialog {
            message: &format!("Building container for environment {}...", env.name()),
            help_message: None,
            typed: Spinner::new(|| env.build_container(&flox)),
        }
        .spin()?;

        Dialog {
            message: &format!("Writing container to '{output_name}'"),
            help_message: None,
            typed: Spinner::new(|| builder.stream_container(output)),
        }
        .spin()?;

        message::created(format!("Container written to '{output_name}'"));
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use flox_rust_sdk::flox::test_helpers::{
        flox_instance,
        flox_instance_with_global_lock_and_floxhub,
    };
    use flox_rust_sdk::models::environment::managed_environment::test_helpers::{
        mock_managed_environment,
        unusable_mock_managed_environment,
    };
    use flox_rust_sdk::models::environment::test_helpers::MANIFEST_INCOMPATIBLE_SYSTEM;
    use flox_rust_sdk::models::pkgdb::error_codes::{self, PACKAGE_BUILD_FAILURE};
    use flox_rust_sdk::models::pkgdb::{CallPkgDbError, PkgDbError};
    use tempfile::tempdir_in;

    use super::*;

    fn incompatible_system_result() -> Result<(), EnvironmentError2> {
        Err(EnvironmentError2::Core(
            CoreEnvironmentError::LockedManifest(LockedManifestError::BuildEnv(
                CallPkgDbError::PkgDbError(PkgDbError {
                    exit_code: error_codes::LOCKFILE_INCOMPATIBLE_SYSTEM,
                    category_message: "category_message".to_string(),
                    context_message: None,
                }),
            )),
        ))
    }

    fn incompatible_package_result() -> Result<(), EnvironmentError2> {
        Err(EnvironmentError2::Core(
            CoreEnvironmentError::LockedManifest(LockedManifestError::BuildEnv(
                CallPkgDbError::PkgDbError(PkgDbError {
                    exit_code: PACKAGE_BUILD_FAILURE,
                    category_message: "category_message".to_string(),
                    context_message: None,
                }),
            )),
        ))
    }

    #[test]
    fn ensure_valid_mock_incompatible_system_result() {
        match incompatible_system_result() {
            Err(EnvironmentError2::Core(core_err)) if core_err.is_incompatible_system_error() => {},
            _ => panic!(),
        }
    }

    #[test]
    fn ensure_valid_mock_incompatible_package_result() {
        match incompatible_package_result() {
            Err(EnvironmentError2::Core(core_err)) if core_err.is_incompatible_package_error() => {
            },
            _ => panic!(),
        }
    }

    // Pulling an environment without packages for the current platform should
    // fail with an error and remove the pulled environment
    #[test]
    fn test_handle_pull_result_1() {
        let (flox, _temp_dir_handle) = flox_instance();

        let dot_flox_path = tempdir_in(&flox.temp_dir).unwrap().into_path();

        assert!(Pull::handle_pull_result(
            &flox,
            incompatible_system_result(),
            &dot_flox_path,
            false,
            unusable_mock_managed_environment(),
            None
        )
        .unwrap_err()
        .to_string()
        .contains("This environment is not yet compatible with your system"));

        assert!(!dot_flox_path.exists());
    }

    /// Pulling an environment without packages for the current platform should
    /// succeed if force is passed
    #[test]
    fn test_handle_pull_result_2() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock_and_floxhub(&owner);

        let dot_flox_path = tempdir_in(&flox.temp_dir).unwrap().into_path();

        Pull::handle_pull_result(
            &flox,
            incompatible_system_result(),
            &dot_flox_path,
            true,
            mock_managed_environment(&flox, MANIFEST_INCOMPATIBLE_SYSTEM, owner),
            None,
        )
        .unwrap();

        assert!(dot_flox_path.exists());
    }

    /// Pulling an environment without packages for the current platform should
    /// prompt about adding system
    /// When the user does not confirm, the environment should be removed
    #[test]
    fn test_handle_pull_result_3() {
        let (flox, _temp_dir_handle) = flox_instance();

        let dot_flox_path = tempdir_in(&flox.temp_dir).unwrap().into_path();

        assert!(Pull::handle_pull_result(
            &flox,
            incompatible_system_result(),
            &dot_flox_path,
            false,
            unusable_mock_managed_environment(),
            Some(QueryFunctions {
                query_add_system: |_| Ok(false),
                query_ignore_build_errors: || panic!(),
            })
        )
        .unwrap_err()
        .to_string()
        .contains("Did not pull the environment"));

        assert!(!dot_flox_path.exists());
    }

    /// Pulling an environment without packages for the current platform should
    /// prompt about adding system
    /// When the user confirms, the environment should not be removed
    #[test]
    fn test_handle_pull_result_4() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock_and_floxhub(&owner);

        let dot_flox_path = tempdir_in(&flox.temp_dir).unwrap().into_path();

        Pull::handle_pull_result(
            &flox,
            incompatible_system_result(),
            &dot_flox_path,
            false,
            mock_managed_environment(&flox, MANIFEST_INCOMPATIBLE_SYSTEM, owner),
            Some(QueryFunctions {
                query_add_system: |_| Ok(true),
                query_ignore_build_errors: || panic!(),
            }),
        )
        .unwrap();

        assert!(dot_flox_path.exists());
    }

    /// Pulling an environment with a package that is not available for the
    /// current platform should prompt to ignore the error.
    /// When answering no, an error should be shown and the environment should be removed.
    #[test]
    fn test_handle_pull_result_5() {
        let (flox, _temp_dir_handle) = flox_instance();

        let dot_flox_path = tempdir_in(&flox.temp_dir).unwrap().into_path();

        assert!(Pull::handle_pull_result(
            &flox,
            incompatible_package_result(),
            &dot_flox_path,
            false,
            unusable_mock_managed_environment(),
            Some(QueryFunctions {
                query_add_system: |_| panic!(),
                query_ignore_build_errors: || Ok(false),
            })
        )
        .unwrap_err()
        .to_string()
        .contains("Did not pull the environment"));

        assert!(!dot_flox_path.exists());
    }

    /// Pulling an environment with a package that is not available for the
    /// current platform should prompt to ignore the error.
    /// When answering yes, the environment should not be removed.
    #[test]
    fn test_handle_pull_result_6() {
        let (flox, _temp_dir_handle) = flox_instance();

        let dot_flox_path = tempdir_in(&flox.temp_dir).unwrap().into_path();

        Pull::handle_pull_result(
            &flox,
            incompatible_package_result(),
            &dot_flox_path,
            false,
            unusable_mock_managed_environment(),
            Some(QueryFunctions {
                query_add_system: |_| panic!(),
                query_ignore_build_errors: || Ok(true),
            }),
        )
        .unwrap();

        assert!(dot_flox_path.exists());
    }
}
