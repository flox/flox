use std::env;
use std::fs::File;
use std::io::stdin;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::{EnvironmentName, Flox};
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironmentError,
    SyncToGenerationResult,
};
use flox_rust_sdk::models::environment::{
    CoreEnvironmentError,
    EditResult,
    Environment,
    EnvironmentError,
};
use itertools::Itertools;
use log::debug;
use tracing::instrument;

use super::{
    activated_environments,
    environment_description,
    environment_select,
    maybe_migrate_environment_to_v1,
    EnvironmentSelect,
    MigrationError,
    UninitializedEnvironment,
};
use crate::commands::{ensure_floxhub_token, ConcreteEnvironment, EnvironmentSelectError};
use crate::subcommand_metric;
use crate::utils::dialog::{Confirm, Dialog, Spinner};
use crate::utils::errors::{
    apply_doc_link_for_unsupported_packages,
    format_core_error,
    format_migration_error,
};
use crate::utils::message;

// Edit declarative environment configuration
#[derive(Bpaf, Clone)]
pub struct Edit {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(external(edit_action), fallback(EditAction::EditManifest{file: None}))]
    action: EditAction,
}
#[derive(Bpaf, Clone)]
pub enum EditAction {
    EditManifest {
        /// Replace environment manifest with that in <file>
        #[bpaf(long, short, argument("file"))]
        file: Option<PathBuf>,
    },

    Rename {
        /// Rename the environment to <name>
        #[bpaf(long, short, argument("name"))]
        name: EnvironmentName,
    },

    Sync {
        /// Create a new generation from the current local environment
        ///
        /// (Only available for managed environments)
        #[bpaf(long, short)]
        sync: (),
    },

    Reset {
        /// Reset the environment to the current generation
        ///
        /// (Only available for managed environments)
        #[bpaf(long, short)]
        reset: (),
    },
}

impl Edit {
    #[instrument(name = "edit", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        subcommand_metric!("edit");

        let detected_environment = match self.environment.detect_concrete_environment(&flox, "Edit")
        {
            Ok(concrete_env) => concrete_env,
            Err(EnvironmentSelectError::Anyhow(e)) => Err(e)?,
            Err(e) => Err(e)?,
        };
        // Ensure the user is logged in for the following remote operations
        if let ConcreteEnvironment::Remote(_) = detected_environment {
            ensure_floxhub_token(&mut flox).await?;
        };

        match self.action {
            EditAction::EditManifest { file } => {
                // TODO: differentiate between interactive edits and replacement
                let span = tracing::info_span!("edit_file");
                let _guard = span.enter();

                let contents = Self::provided_manifest_contents(file)?;

                if let ConcreteEnvironment::Managed(ref environment) = detected_environment {
                    if environment.has_local_changes(&flox)? && contents.is_none() {
                        bail!(ManagedEnvironmentError::CheckoutOutOfSync)
                    }
                };

                // TODO: we have various functionality spread across
                // UninitializedEnvironment, ConcreteEnvironment, and
                // Environment.
                // UninitializedEnvironment is used to compare to what
                // environments are active.
                // description can't currently be derived from an Environment
                // but is used for messages.
                // Environment is what we'll actually use to perform the edit.
                let active_environment =
                    UninitializedEnvironment::from_concrete_environment(&detected_environment)?;
                let description = environment_description(&detected_environment)?;
                let mut environment = detected_environment.into_dyn_environment();

                Self::edit_manifest(
                    &flox,
                    &mut *environment,
                    active_environment,
                    description,
                    contents,
                )
                .await?
            },
            EditAction::Rename { name } => {
                // TODO: we could migrate environment to v1 if we wanted to
                let span = tracing::info_span!("rename");
                let _guard = span.enter();
                if let ConcreteEnvironment::Path(mut environment) = detected_environment {
                    let old_name = environment.name();
                    if name == old_name {
                        bail!("environment already named '{name}'");
                    }
                    environment.rename(name.clone())?;
                    message::updated(format!("renamed environment '{old_name}' to '{name}'"));
                } else {
                    // todo: handle remote environments in the future
                    bail!("Cannot rename environments on FloxHub");
                }
            },

            EditAction::Sync { .. } => {
                let span = tracing::info_span!("sync");
                let _guard = span.enter();
                let description = environment_description(&detected_environment)?;
                let ConcreteEnvironment::Managed(mut environment) = detected_environment else {
                    bail!("Cannot sync local or remote environments.");
                };

                maybe_migrate_environment_to_v1(&flox, &mut environment, &description).await?;

                let sync_result = environment.create_generation_from_local_env(&flox)?;
                match sync_result {
                    SyncToGenerationResult::UpToDate => message::plain("No local changes to sync."),
                    SyncToGenerationResult::Synced => {
                        message::updated("Environment successfully synced to a new generation.")
                    },
                }
            },

            EditAction::Reset { .. } => {
                let span = tracing::info_span!("reset");
                let _guard = span.enter();
                let ConcreteEnvironment::Managed(mut environment) = detected_environment else {
                    bail!("Cannot reset local or remote environments.");
                };

                environment.reset_local_env_to_current_generation(&flox)?;

                Dialog {
                    message: "Building environment",
                    help_message: None,
                    typed: Spinner::new(|| environment.build(&flox)),
                }
                .spin()?;

                message::updated("Environment changes reset to current generation.");
            },
        }

        Ok(())
    }

    // TODO: having to pass environment + active_environment + description
    // instead of just environment is a pain
    async fn edit_manifest(
        flox: &Flox,
        environment: &mut dyn Environment,
        active_environment: UninitializedEnvironment,
        description: String,
        contents: Option<String>,
    ) -> Result<()> {
        match maybe_migrate_environment_to_v1(flox, environment, &description).await {
            Ok(_) => (),
            e @ Err(MigrationError::MigrationCancelled) => e?,

            // If the user said they wanted an upgrade and it failed, print why but don't fail
            Err(e @ MigrationError::ConfirmedUpgradeFailed(_)) => {
                message::warning(format_migration_error(&e));
            },
            // Swallow other migration errors because edit is the only way to fix them.
            // Don't print anything if there's an error, because the editor will
            // open too fast for the user to see it.
            Err(_) => (),
        };

        let result = match contents {
            // If provided with the contents of a manifest file, either via a path to a file or via
            // contents piped to stdin, use those contents to try building the environment.
            Some(new_manifest) => environment
                .edit(flox, new_manifest)
                .map_err(apply_doc_link_for_unsupported_packages)?,
            // If not provided with new manifest contents, let the user edit the file directly
            // via $EDITOR or $VISUAL (as long as `flox edit` was invoked interactively).
            None => Self::interactive_edit(flox, environment).await?,
        };

        // outside the match to avoid rustfmt falling on its face
        let reactivate_required_note = indoc::indoc! {"
            Your manifest has changes that cannot be automatically applied.

            Please 'exit' the environment and run 'flox activate' to see these changes.
       "};

        match result {
            EditResult::Unchanged => {
                message::warning("No changes made to environment.");
            },
            EditResult::ReActivateRequired { .. }
                if activated_environments().is_active(&active_environment) =>
            {
                message::warning(reactivate_required_note)
            },
            EditResult::ReActivateRequired { .. } => {
                message::updated("Environment successfully updated.")
            },
            EditResult::Success { .. } => message::updated("Environment successfully updated."),
        }
        Ok(())
    }

    /// Interactively edit the manifest file
    async fn interactive_edit(
        flox: &Flox,
        environment: &mut dyn Environment,
    ) -> Result<EditResult> {
        if !Dialog::can_prompt() {
            bail!("Can't edit interactively in non-interactive context")
        }

        let editor = Self::determine_editor()?;

        // Make a copy of the manifest for the user to edit so failed edits aren't left in
        // the original manifest. You can't put creation/cleanup inside the `edited_manifest_contents`
        // method because the temporary manifest needs to stick around in case the user wants
        // or needs to make successive edits without starting over each time.
        let tmp_manifest = tempfile::Builder::new()
            .prefix("manifest.")
            .suffix(".toml")
            .tempfile_in(&flox.temp_dir)?;
        std::fs::write(&tmp_manifest, environment.manifest_content(flox)?)?;

        let should_continue_dialog = Dialog {
            message: "Continue editing?",
            help_message: Default::default(),
            typed: Confirm {
                default: Some(true),
            },
        };

        // Let the user keep editing the file until the build succeeds or the user
        // decides to stop.
        loop {
            let new_manifest = Edit::edited_manifest_contents(&tmp_manifest, &editor)?;

            let result = Dialog {
                message: "Building environment to validate edit...",
                help_message: None,
                typed: Spinner::new(|| environment.edit(flox, new_manifest.clone())),
            }
            .spin()
            .map_err(apply_doc_link_for_unsupported_packages);

            match Self::make_interactively_recoverable(result)? {
                Ok(result) => return Ok(result),

                // for recoverable errors, prompt the user to continue editing
                Err(e) => {
                    message::error(format_core_error(&e));

                    if !Dialog::can_prompt() {
                        bail!("Can't prompt to continue editing in non-interactive context");
                    }
                    if !should_continue_dialog.clone().prompt().await? {
                        bail!("Environment editing cancelled");
                    }
                },
            }
        }
    }

    /// Returns `Ok` if the edit result is successful or recoverable, `Err` otherwise
    fn make_interactively_recoverable(
        result: Result<EditResult, EnvironmentError>,
    ) -> Result<Result<EditResult, CoreEnvironmentError>, EnvironmentError> {
        match result {
            Err(EnvironmentError::Core(e @ CoreEnvironmentError::LockedManifest(_)))
            | Err(EnvironmentError::Core(e @ CoreEnvironmentError::DeserializeManifest(_)))
            | Err(EnvironmentError::Core(e @ CoreEnvironmentError::Version0NotSupported)) => {
                Ok(Err(e))
            },
            Err(e) => Err(e),
            Ok(result) => Ok(Ok(result)),
        }
    }

    /// Determines the editor to use for interactive editing
    ///
    /// If $EDITOR or $VISUAL is set, use that. Otherwise, try to find a known editor in $PATH.
    /// The known editor selected is the first one found in $PATH from the following list:
    ///
    ///   vim, vi, nano, emacs.
    fn determine_editor() -> Result<PathBuf> {
        let editor = std::env::var("EDITOR").or(std::env::var("VISUAL")).ok();

        if let Some(editor) = editor {
            return Ok(PathBuf::from(editor));
        }

        let path_var = env::var("PATH").context("$PATH not set")?;

        let (path, editor) = env::split_paths(&path_var)
            .cartesian_product(["vim", "vi", "nano", "emacs"])
            .find(|(path, editor)| path.join(editor).exists())
            .context("no known editor found in $PATH")?;

        debug!("Using editor {:?} from {:?}", editor, path);

        Ok(path.join(editor))
    }

    /// Retrieves the new manifest file contents if a new manifest file was provided
    fn provided_manifest_contents(file: Option<PathBuf>) -> Result<Option<String>> {
        if let Some(ref file) = file {
            let mut file: Box<dyn std::io::Read + Send> = if file == Path::new("-") {
                Box::new(stdin())
            } else {
                Box::new(File::open(file)?)
            };

            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            Ok(Some(contents))
        } else {
            Ok(None)
        }
    }

    /// Gets a new set of manifest contents after a user edits the file
    fn edited_manifest_contents(
        path: impl AsRef<Path>,
        editor: impl AsRef<Path>,
    ) -> Result<String> {
        let mut command = Command::new(editor.as_ref());
        command.arg(path.as_ref());

        let child = command.spawn().context("editor command failed")?;
        let _ = child.wait_with_output().context("editor command failed")?;

        let contents = std::fs::read_to_string(path)?;
        Ok(contents)
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance_with_optional_floxhub_and_client;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment;
    use flox_rust_sdk::models::environment::test_helpers::MANIFEST_V0_FIELDS;
    use flox_rust_sdk::models::lockfile::LockedManifestError;
    use indoc::indoc;
    use serde::de::Error;

    use super::*;

    /// successful edit returns value that will end the loop
    #[test]
    fn test_recover_edit_loop_result_success() {
        let result = Ok(EditResult::Unchanged);

        Edit::make_interactively_recoverable(result)
            .expect("should return Ok")
            .expect("should return Ok");
    }

    /// errors parsing the manifest are recoverable
    #[test]
    fn test_recover_edit_loop_result_bad_manifest() {
        let result = Err(EnvironmentError::Core(
            CoreEnvironmentError::DeserializeManifest(toml::de::Error::custom("msg")),
        ));

        Edit::make_interactively_recoverable(result)
            .expect("should be recoverable")
            .expect_err("should return recoverable Err");
    }

    /// errors locking the manifest are recoverable
    #[test]
    fn test_recover_edit_loop_result_locking() {
        let result = Err(EnvironmentError::Core(
            CoreEnvironmentError::LockedManifest(LockedManifestError::EmptyPage),
        ));

        Edit::make_interactively_recoverable(result)
            .expect("should be recoverable")
            .expect_err("should return recoverable err");
    }

    /// Error due to manifest version 0 is recoverable
    #[test]
    fn test_recover_edit_loop_result_version_0() {
        let result = Err(EnvironmentError::Core(
            CoreEnvironmentError::Version0NotSupported,
        ));

        Edit::make_interactively_recoverable(result)
            .expect("should be recoverable")
            .expect_err("should return recoverable err");
    }

    /// other errors are not recoverable and should be returned as-is
    #[test]
    fn test_recover_edit_loop_result_other_error() {
        let result = Err(EnvironmentError::Core(
            CoreEnvironmentError::CatalogClientMissing,
        ));

        Edit::make_interactively_recoverable(result).expect_err("should return unhandled Err");
    }

    /// Given a v0 manifest that can be migrated and v0 contents, the migration
    /// should succeed,
    /// but the edit should fail.
    #[tokio::test]
    async fn migration_successful_migration_unsuccessful_edit() {
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub_and_client(None, true);
        let concrete_environment = ConcreteEnvironment::Path(new_path_environment(&flox, ""));
        let new_contents = indoc! {r#"
            [options]
            allow.broken = false
            "#};

        let active_environment =
            UninitializedEnvironment::from_concrete_environment(&concrete_environment).unwrap();
        let description = environment_description(&concrete_environment).unwrap();
        let mut environment = concrete_environment.into_dyn_environment();

        let err = Edit::edit_manifest(
            &flox,
            &mut *environment,
            active_environment,
            description,
            Some(new_contents.to_string()),
        )
        .await
        .unwrap_err()
        .downcast::<EnvironmentError>()
        .unwrap();

        assert!(matches!(
            err,
            EnvironmentError::Core(CoreEnvironmentError::Version0NotSupported)
        ));

        let actual_contents = environment.manifest_content(&flox).unwrap();
        assert_eq!(actual_contents, "version = 1\n");
    }

    /// Given a v0 manifest that cannot be migrated and v0 contents, the migration
    /// should fail,
    /// and the edit should fail.
    #[tokio::test]
    async fn migration_unsuccessful_migration_unsuccessful_edit() {
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub_and_client(None, true);

        let concrete_environment =
            ConcreteEnvironment::Path(new_path_environment(&flox, MANIFEST_V0_FIELDS));

        let active_environment =
            UninitializedEnvironment::from_concrete_environment(&concrete_environment).unwrap();
        let description = environment_description(&concrete_environment).unwrap();
        let mut environment = concrete_environment.into_dyn_environment();

        let new_contents = indoc! {r#"
            [options]
            allow.broken = false
            "#};

        let err = Edit::edit_manifest(
            &flox,
            &mut *environment,
            active_environment,
            description,
            Some(new_contents.to_string()),
        )
        .await
        .unwrap_err()
        .downcast::<EnvironmentError>()
        .unwrap();

        assert!(matches!(
            err,
            EnvironmentError::Core(CoreEnvironmentError::Version0NotSupported)
        ));

        let actual_contents = environment.manifest_content(&flox).unwrap();
        assert!(!actual_contents.contains("version = 1"));
    }

    /// Given a v0 manifest that cannot be migrated and v1 contents, the migration
    /// should fail,
    /// but the edit should succeed.
    #[tokio::test]
    async fn migration_unsuccessful_migration_successful_edit() {
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub_and_client(None, true);

        let concrete_environment =
            ConcreteEnvironment::Path(new_path_environment(&flox, MANIFEST_V0_FIELDS));

        let active_environment =
            UninitializedEnvironment::from_concrete_environment(&concrete_environment).unwrap();
        let description = environment_description(&concrete_environment).unwrap();
        let mut environment = concrete_environment.into_dyn_environment();

        let new_contents = indoc! {r#"
            version = 1

            [options]
            allow.broken = false
            "#};

        Edit::edit_manifest(
            &flox,
            &mut *environment,
            active_environment,
            description,
            Some(new_contents.to_string()),
        )
        .await
        .unwrap();

        // TODO: would be nice to make an assertion about
        // `Failed to migrate environment to version 1` being printed.

        let actual_contents = environment.manifest_content(&flox).unwrap();
        assert!(actual_contents.contains("version = 1"));
    }

    /// Given a v0 manifest that can be migrated and v1 contents, the migration
    /// should succeed,
    /// and the edit should succeed.
    #[tokio::test]
    async fn migration_successful_migration_successful_edit() {
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub_and_client(None, true);
        let old_contents = indoc! {r#"
            [options]
            allow.broken = false
            "#};

        let concrete_environment =
            ConcreteEnvironment::Path(new_path_environment(&flox, old_contents));

        let active_environment =
            UninitializedEnvironment::from_concrete_environment(&concrete_environment).unwrap();
        let description = environment_description(&concrete_environment).unwrap();
        let mut environment = concrete_environment.into_dyn_environment();

        let new_contents = indoc! {r#"
            version = 1

            [options]
            allow.broken = false
            "#};

        Edit::edit_manifest(
            &flox,
            &mut *environment,
            active_environment,
            description,
            Some(new_contents.to_string()),
        )
        .await
        .unwrap();

        let actual_contents = environment.manifest_content(&flox).unwrap();
        assert!(actual_contents.contains("version = 1"));
    }
}
