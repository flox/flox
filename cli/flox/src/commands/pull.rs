use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::{EnvironmentRef, Flox};
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironment,
    ManagedEnvironmentError,
    PullResult,
};
use flox_rust_sdk::models::environment::{
    CoreEnvironmentError,
    DotFlox,
    Environment,
    EnvironmentError,
    EnvironmentPointer,
    ManagedPointer,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
};
use flox_rust_sdk::models::manifest;
use indoc::{formatdoc, indoc};
use log::debug;
use toml_edit::DocumentMut;
use tracing::instrument;

use super::services::warn_manifest_changes_for_services;
use super::{open_path, ConcreteEnvironment};
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Select, Spinner};
use crate::utils::errors::{display_chain, format_core_error};
use crate::utils::message;

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
    /// Directory in which to create a managed environment,
    /// or directory that already contains a managed environment
    /// (default: current directory)
    #[bpaf(long, short, argument("path"))]
    dir: Option<PathBuf>,

    /// Forcibly pull the environment
    /// When pulling a new environment, adds the system to the manifest if the lockfile is incompatible
    /// and ignores eval and build errors.
    /// When pulling an existing environment, overrides local changes.
    #[bpaf(long, short)]
    force: bool,

    /// Create a copy of the upstream environment.
    #[bpaf(short, long)]
    copy: bool,

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
                let start_message = format!(
                    "⬇️  Remote: pulling and building {env_ref} from {host} into {into_dir}",
                    env_ref = &remote,
                    host = flox.floxhub.base_url(),
                    into_dir = if let Some(dir) = self.dir.as_deref() {
                        format!("{}", dir.display())
                    } else {
                        "the current directory".to_string()
                    }
                );

                // FIXME: this could panic if the current directory is deleted between
                //        calling `flox` and running this line
                let dir = self.dir.unwrap_or_else(|| std::env::current_dir().unwrap());

                debug!("Resolved user intent: pull {remote:?} into {dir:?}");

                Self::pull_new_environment(
                    &flox,
                    dir,
                    remote,
                    self.copy,
                    self.force,
                    &start_message,
                )?;
            },
            PullSelect::Existing {} => {
                let dir = self.dir.unwrap_or_else(|| std::env::current_dir().unwrap());

                debug!("Resolved user intent: pull changes for environment found in {dir:?}");

                let pointer = {
                    let p = DotFlox::open_in(&dir)?.pointer;
                    match p {
                        EnvironmentPointer::Managed(managed_pointer) => managed_pointer,
                        EnvironmentPointer::Path(_) => bail!("Cannot pull into a path environment"),
                    }
                };

                if self.copy {
                    debug!("`--copy` provided. converting to path environment, skipping pull");
                    let env = ManagedEnvironment::open(&flox, pointer.clone(), dir.join(DOT_FLOX))?;
                    env.into_path_environment(&flox)?;

                    message::created(formatdoc! {"
                        Created path environment from {owner}/{name}.
                    ", owner = pointer.owner, name = pointer.name});
                    return Ok(());
                }

                Self::pull_existing_environment(
                    &flox,
                    dir.join(DOT_FLOX),
                    pointer.clone(),
                    self.force,
                )?;
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
    ) -> Result<(), EnvironmentError> {
        let mut env = ManagedEnvironment::open(flox, pointer.clone(), dot_flox_path)?;

        let pull_message = format!(
            "⬇️  Remote: pulling {owner}/{name} from {floxhub_host}",
            owner = pointer.owner,
            name = pointer.name,
            floxhub_host = flox.floxhub.base_url()
        );

        let state = Dialog {
            message: &pull_message,
            help_message: None,
            typed: Spinner::new(|| env.pull(flox, force)),
        }
        .spin()?;

        match state {
            PullResult::Updated => {
                // Only build if the environment was updated
                //
                // Build errors are _not_ handled here
                // as it is assumed that environments were validated during push.
                Dialog {
                    message: "🛠️  Building the environment",
                    help_message: None,
                    typed: Spinner::new(|| {
                        // The pulled generation already has a lock,
                        // so we can skip locking.
                        let store_paths = env.build(flox)?;
                        env.link(&store_paths)
                    }),
                }
                .spin()?;

                message::updated(formatdoc! {"
                    Pulled {owner}/{name} from {floxhub_host}{suffix}

                    You can activate this environment with 'flox activate'
                    ",
                    owner = pointer.owner, name = pointer.name,
                    floxhub_host = flox.floxhub.base_url(),
                    suffix = if force { " (forced)" } else { "" }
                });

                warn_manifest_changes_for_services(flox, &env);
            },
            PullResult::UpToDate => {
                message::warning(formatdoc! {"
                            {owner}/{name} is already up to date.
                        ", owner = pointer.owner, name = pointer.name});
            },
        }

        Ok(())
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
        env_path: PathBuf,
        env_ref: EnvironmentRef,
        copy: bool,
        force: bool,
        message: &str,
    ) -> Result<()> {
        let dot_flox_path = env_path.join(DOT_FLOX);
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
                            "Failed to remove existing .flox directory at {dot_flox_path:?}"
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
            .map_err(Self::handle_error);

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
            typed: Spinner::new(|| {
                // The pulled generation already has a lock,
                // so we can skip locking.
                let store_paths = env.build(flox)?;
                env.link(&store_paths)
            }),
        }
        .spin();

        let resolution = Self::handle_pull_result(
            flox,
            result,
            &dot_flox_path,
            force,
            &mut env,
            Dialog::can_prompt().then_some(QueryFunctions {
                query_add_system: Self::query_add_system,
                query_ignore_build_errors: Self::query_ignore_build_errors,
            }),
        )?;

        if copy {
            debug!("Converting environment to path environment");
            if let Err(e) = env.into_path_environment(flox) {
                fs::remove_dir_all(&dot_flox_path)
                    .context("Could not clean up .flox/ directory")?;
                bail!(e);
            }
        }

        let message_lead = if copy {
            format!("Created path environment from {env_ref}.")
        } else {
            format!(
                "Pulled {env_ref} from {floxhub_host}.",
                floxhub_host = flox.floxhub.base_url()
            )
        };

        match resolution {
            PullResultResolutionContext::Success(completed) => {
                message::created(formatdoc! {"
                    {message_lead}

                    {completed}
                "});
            },
            PullResultResolutionContext::Warning(warning) => {
                message::warning(formatdoc! {"
                    {message_lead}

                    {warning}
                "});
            },
        }

        Ok(())
    }

    /// Helper function for [Self::pull_new_environment] that can be unit tested.
    ///
    /// A value of [None] for query_functions represents when the user cannot be prompted.
    /// [Some] represents when the user should be prompted with the provided functions.
    fn handle_pull_result<T>(
        flox: &Flox,
        result: Result<T, EnvironmentError>,
        dot_flox_path: &PathBuf,
        force: bool,
        env: &mut ManagedEnvironment,
        query_functions: Option<QueryFunctions>,
    ) -> Result<PullResultResolutionContext> {
        let completed = indoc! {"
           You can activate this environment with 'flox activate'\
        "};

        let Err(err) = result else {
            return Ok(PullResultResolutionContext::Success(completed));
        };

        match err {
            // Environment is not compatible with the current system
            EnvironmentError::Core(e) if e.is_incompatible_system_error() => {
                let hint = formatdoc! {"
                    Use 'flox pull --force' to add your system to the manifest.
                    For more on managing systems for your environment, visit the documentation:
                    https://flox.dev/docs/tutorials/multi-arch-environments
                "};

                if !force && query_functions.is_none() {
                    fs::remove_dir_all(dot_flox_path)
                        .context("Could not clean up .flox/ directory")?;
                    bail!(formatdoc! {"
                        This environment is not yet compatible with your system ({system}).

                        {hint}",
                        system = flox.system
                    });
                }

                // Query functions will return `Ok(false)` if the user chose to abort the pull.
                // The unwrap() is only reached if !force,
                // and we return above if !force and query_functions.is_none()
                let stop = !force && !(query_functions.unwrap().query_add_system)(&flox.system)?;

                if stop {
                    // prompt available, user chose to abort
                    fs::remove_dir_all(dot_flox_path)
                        .context("Could not clean up .flox/ directory")?;
                    bail!(formatdoc! {"
                        Did not pull the environment.

                        {hint}
                    "});
                }

                let manifest_with_current_system = Self::amend_current_system(env, flox)?;
                let rebuild_with_current_system = Dialog {
                    message: "Adding your system to the manifest and validating the environment.",
                    help_message: None,
                    typed: Spinner::new(|| {
                        env.edit_unsafe(flox, manifest_with_current_system.to_string())
                    }),
                }
                .spin()?;

                match rebuild_with_current_system {
                    Err(broken_error) => {
                        message::warning(format!("{err:#}", err = anyhow!(broken_error)));

                        let message_with_warning = indoc! {"
                            Modified the manifest to include your system but could not build.
                            Use 'flox edit' to address issues before activating.\
                        "};

                        Ok(PullResultResolutionContext::Warning(message_with_warning))
                    },
                    Ok(_) => Ok(PullResultResolutionContext::Success(completed)),
                }
            },

            // Failed to _build_ the environment due to an incompatible package
            EnvironmentError::Core(ref core_err @ CoreEnvironmentError::BuildEnv(_))
                if core_err.is_incompatible_package_error() =>
            {
                debug!(
                    "environment contains package incompatible with the current system: {err}",
                    err = display_chain(core_err)
                );

                let pkgdb_error = format_core_error(core_err);

                if !force && query_functions.is_none() {
                    fs::remove_dir_all(dot_flox_path)
                        .context("Could not clean up .flox/ directory")?;
                    bail!("{pkgdb_error}");
                }

                message::error(pkgdb_error);

                // The unwrap() is only reached if !force,
                // and we return above if !force and query_functions.is_none()
                if force || (query_functions.unwrap().query_ignore_build_errors)()? {
                    let message_with_warning = indoc! {"
                        Could not build environment.
                        Use 'flox edit' to address issues before activating.\
                    "};
                    Ok(PullResultResolutionContext::Warning(message_with_warning))
                } else {
                    fs::remove_dir_all(dot_flox_path)
                        .context("Could not clean up .flox/ directory")?;
                    bail!("Did not pull the environment.");
                }
            },

            e => {
                fs::remove_dir_all(dot_flox_path).context("Could not clean up .flox/ directory")?;
                bail!(e)
            },
        }
    }

    /// prompt the user whether to automatically add their system to the manifest
    ///
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
        env: &impl Environment,
        flox: &Flox,
    ) -> Result<DocumentMut, anyhow::Error> {
        manifest::add_system(&env.manifest_contents(flox)?, &flox.system)
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

    fn handle_error(err: ManagedEnvironmentError) -> anyhow::Error {
        match err {
            ManagedEnvironmentError::AccessDenied => {
                let message = "You do not have permission to pull this environment";
                anyhow::Error::msg(message)
            },
            ManagedEnvironmentError::Diverged => {
                let message = "The environment has diverged from the remote version";
                anyhow::Error::msg(message)
            },
            ManagedEnvironmentError::UpstreamNotFound {
                env_ref,
                upstream: _,
                user,
            } => {
                let by_current_user = user
                    .map(|u| u == env_ref.owner().as_str())
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

/// Additional (user facing) context for the result of [Pull::handle_pull_result].
///
/// This is used to construct the message to show to the user
/// upon completion of the pull operation.
///
/// Resolving pull errors can succeed, fail,
/// or fail where the error is eventually resolved by the user.
/// The latter case is represents errors that happen
/// when migrating or adding missing systems to the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, derive_more::Display)]
enum PullResultResolutionContext {
    /// The pull operation was successful and the environment is ready to use
    Success(&'static str),
    /// The pull operation was successful
    /// but the environment might require manual intervention before it can be used
    Warning(&'static str),
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use flox_rust_sdk::models::environment::managed_environment::test_helpers::{
        mock_managed_environment,
        unusable_mock_managed_environment,
    };
    use flox_rust_sdk::models::environment::test_helpers::MANIFEST_INCOMPATIBLE_SYSTEM;
    use flox_rust_sdk::models::pkgdb::error_codes::PACKAGE_BUILD_FAILURE;
    use flox_rust_sdk::models::pkgdb::PkgDbError;
    use flox_rust_sdk::providers::buildenv::BuildEnvError;
    use tempfile::tempdir_in;

    use super::*;

    fn incompatible_system_result() -> Result<(), EnvironmentError> {
        Err(EnvironmentError::Core(CoreEnvironmentError::BuildEnv(
            BuildEnvError::LockfileIncompatible { systems: vec![] },
        )))
    }

    fn incompatible_package_result() -> Result<(), EnvironmentError> {
        Err(EnvironmentError::Core(CoreEnvironmentError::BuildEnv(
            BuildEnvError::Realise(PkgDbError {
                exit_code: PACKAGE_BUILD_FAILURE,
                category_message: "category_message".to_string(),
                context_message: None,
            }),
        )))
    }

    #[test]
    fn ensure_valid_mock_incompatible_system_result() {
        match incompatible_system_result() {
            Err(EnvironmentError::Core(core_err)) if core_err.is_incompatible_system_error() => {},
            _ => panic!(),
        }
    }

    #[test]
    fn ensure_valid_mock_incompatible_package_result() {
        match incompatible_package_result() {
            Err(EnvironmentError::Core(core_err)) if core_err.is_incompatible_package_error() => {},
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
            &mut unusable_mock_managed_environment(),
            None
        )
        .unwrap_err()
        .to_string()
        .contains("This environment is not yet compatible with your system"));

        assert!(!dot_flox_path.exists());
    }

    /// Pulling an environment without packages for the current platform should
    /// succeed if force is passed
    // handle_pull_result() calls spin() which depends on tokio
    #[tokio::test]
    async fn test_handle_pull_result_2() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let dot_flox_path = tempdir_in(&flox.temp_dir).unwrap().into_path();

        Pull::handle_pull_result(
            &flox,
            incompatible_system_result(),
            &dot_flox_path,
            true,
            &mut mock_managed_environment(&flox, MANIFEST_INCOMPATIBLE_SYSTEM, owner),
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
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let dot_flox_path = tempdir_in(&flox.temp_dir).unwrap().into_path();

        assert!(Pull::handle_pull_result(
            &flox,
            incompatible_system_result(),
            &dot_flox_path,
            false,
            &mut mock_managed_environment(&flox, MANIFEST_INCOMPATIBLE_SYSTEM, owner),
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
    // handle_pull_result() calls spin() which depends on tokio
    #[tokio::test]
    async fn test_handle_pull_result_4() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let dot_flox_path = tempdir_in(&flox.temp_dir).unwrap().into_path();

        Pull::handle_pull_result(
            &flox,
            incompatible_system_result(),
            &dot_flox_path,
            false,
            &mut mock_managed_environment(&flox, MANIFEST_INCOMPATIBLE_SYSTEM, owner),
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
            &mut unusable_mock_managed_environment(),
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
            &mut unusable_mock_managed_environment(),
            Some(QueryFunctions {
                query_add_system: |_| panic!(),
                query_ignore_build_errors: || Ok(true),
            }),
        )
        .unwrap();

        assert!(dot_flox_path.exists());
    }
}
