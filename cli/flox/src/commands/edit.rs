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
        #[bpaf(long)]
        reset: (),
    },
}

impl Edit {
    #[instrument(name = "edit", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        subcommand_metric!("edit");

        // Ensure the user is logged in for the following remote operations
        if let EnvironmentSelect::Remote(_) = self.environment {
            ensure_floxhub_token(&mut flox).await?;
        };

        let mut detected_environment =
            match self.environment.detect_concrete_environment(&flox, "Edit") {
                Ok(concrete_env) => concrete_env,
                Err(EnvironmentSelectError::Anyhow(e)) => Err(e)?,
                Err(e) => Err(e)?,
            };

        match self.action {
            EditAction::EditManifest { file } => {
                // TODO: differentiate between interactive edits and replacement
                let span = tracing::info_span!("edit_file");
                let _guard = span.enter();

                let contents = Self::provided_manifest_contents(file)?;

                Self::edit_manifest(&flox, &mut detected_environment, contents).await?
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
                let ConcreteEnvironment::Managed(mut environment) = detected_environment else {
                    bail!("Cannot sync local or remote environments.");
                };

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
                    typed: Spinner::new(|| {
                        // The current generation already has a lock,
                        // so we can skip locking.
                        let store_path = environment.build(&flox)?;
                        environment.link(store_path)
                    }),
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
        environment: &mut ConcreteEnvironment,
        contents: Option<String>,
    ) -> Result<()> {
        if let ConcreteEnvironment::Managed(ref environment) = environment {
            if environment.has_local_changes(flox)? && contents.is_none() {
                bail!(ManagedEnvironmentError::CheckoutOutOfSync)
            }
        };

        match maybe_migrate_environment_to_v1(flox, environment).await {
            Ok(_) => (),
            e @ Err(MigrationError::MigrationCancelled) => e?,

            // If the user said they wanted an upgrade and it failed, print why but don't fail
            // [CoreEnvironmentError::LockForMigration] and [CoreEnvironmentError::MigrateManifest]
            // are handled separately to avoid suggesting the use of `flox edit` within `flox edit`.
            Err(MigrationError::ConfirmedUpgradeFailed(EnvironmentError::Core(
                CoreEnvironmentError::LockForMigration(err),
            ))) => {
                message::warning(format_core_error(&err));
            },
            Err(MigrationError::ConfirmedUpgradeFailed(EnvironmentError::Core(
                CoreEnvironmentError::MigrateManifest(err),
            ))) => {
                message::warning(err.to_string());
            },
            Err(e @ MigrationError::ConfirmedUpgradeFailed(_)) => {
                message::warning(format_migration_error(&e));
            },
            // Swallow other migration errors because edit is the only way to fix them.
            // Don't print anything if there's an error, because the editor will
            // open too fast for the user to see it.
            Err(_) => (),
            // Note: ManagedEnvironmentError::CheckoutOutOfSync case is unreachable here,
            // because it's handled above for clarity.
        };

        let active_environment = UninitializedEnvironment::from_concrete_environment(environment)?;
        let environment = environment.dyn_environment_ref_mut();

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

        let (editor, args) = Self::determine_editor()?;

        // Make a copy of the manifest for the user to edit so failed edits aren't left in
        // the original manifest. You can't put creation/cleanup inside the `edited_manifest_contents`
        // method because the temporary manifest needs to stick around in case the user wants
        // or needs to make successive edits without starting over each time.
        let tmp_manifest = tempfile::Builder::new()
            .prefix("manifest.")
            .suffix(".toml")
            .tempfile_in(&flox.temp_dir)?;
        std::fs::write(&tmp_manifest, environment.manifest_contents(flox)?)?;

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
            let new_manifest = Edit::edited_manifest_contents(&tmp_manifest, &editor, &args)?;

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

    /// Determines the editor to use for interactive editing, based on the environment
    /// Returns the editor and a list of args to pass to the editor
    ///
    /// If $VISUAL or $EDITOR is set, use that.
    /// The editor cannot be an empty string or one that consists of fully Unicode whitespace.
    /// Arguments can be passed and will be split on whitespace.
    /// Otherwise, try to find a known editor in $PATH.
    /// The known editor selected is the first one found in $PATH from the following list:
    ///
    ///   vim, vi, nano, emacs.
    fn determine_editor() -> Result<(PathBuf, Vec<String>)> {
        Self::determine_editor_from_vars(
            env::var("VISUAL").unwrap_or_default(),
            env::var("EDITOR").unwrap_or_default(),
            env::var("PATH").context("$PATH not set")?,
        )
    }

    /// Determines the editor to use for interactive editing, based on passed values
    /// Returns the editor and a list of args to pass to the editor
    fn determine_editor_from_vars(
        visual_var: String,
        editor_var: String,
        path_var: String,
    ) -> Result<(PathBuf, Vec<String>)> {
        let var = if !visual_var.trim().is_empty() {
            visual_var
        } else {
            editor_var
        };
        let mut command = var.split_whitespace();

        let editor = command.next().unwrap_or_default().to_owned();
        let args = command.map(|s| s.to_owned()).collect();

        if !editor.is_empty() {
            debug!("Using configured editor {:?} with args {:?}", editor, args);
            return Ok((PathBuf::from(editor), args));
        }

        let (path, editor) = env::split_paths(&path_var)
            .cartesian_product(["vim", "vi", "nano", "emacs"])
            .find(|(path, editor)| path.join(editor).is_file())
            .context("no known editor found in $PATH")?;

        debug!("Using default editor {:?} from {:?}", editor, path);

        Ok((path.join(editor), vec![]))
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
        args: impl AsRef<Vec<String>>,
    ) -> Result<String> {
        let mut command = Command::new(editor.as_ref());
        if !args.as_ref().is_empty() {
            command.args(args.as_ref());
        }
        command.arg(path.as_ref());

        let child = command.spawn().context("editor command failed")?;
        let _ = child.wait_with_output().context("editor command failed")?;

        let contents = std::fs::read_to_string(path)?;
        Ok(contents)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use flox_rust_sdk::flox::test_helpers::flox_instance_with_optional_floxhub_and_client;
    use flox_rust_sdk::models::environment::managed_environment::test_helpers::mock_managed_environment;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment;
    use flox_rust_sdk::models::environment::test_helpers::MANIFEST_V0_FIELDS;
    use flox_rust_sdk::models::lockfile::LockedManifestError;
    use indoc::indoc;
    use serde::de::Error;
    use tempfile::tempdir;

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

    /// Error due to empty vars and no editor in PATH
    #[test]
    fn test_determine_editor_from_vars_not_found() {
        let visual_var = "".to_owned();
        let editor_var = "".to_owned();

        let tmp1 = tempdir().expect("should create tempdir");
        let tmp2 = tempdir().expect("should create tempdir");
        let tmp3 = tempdir().expect("should create tempdir");

        let path_var = std::env::join_paths([&tmp1, &tmp2, &tmp3].map(|d| d.path()))
            .expect("should path-join tmpdirs")
            .into_string()
            .expect("should convert paths from OsString to String");

        Edit::determine_editor_from_vars(visual_var, editor_var, path_var)
            .expect_err("should error with editor not found");

        assert!(tmp1.path().is_dir());
        assert!(tmp2.path().is_dir());
        assert!(tmp3.path().is_dir());
    }

    /// Default to the first of any editor while traversing PATH
    #[test]
    fn test_determine_editor_from_vars_first_default_editor() {
        let visual_var = "".to_owned();
        let editor_var = "".to_owned();

        let tmp1 = tempdir().expect("should create tempdir");
        let tmp2 = tempdir().expect("should create tempdir");
        let tmp3 = tempdir().expect("should create tempdir");

        let path_var = std::env::join_paths([&tmp1, &tmp2, &tmp3].map(|d| d.path().to_owned()))
            .expect("should path-join tmpdirs")
            .into_string()
            .expect("should convert paths from OsString to String");

        let nano = tmp1.path().join("nano");
        let vim = tmp2.path().join("vim");
        let vi = tmp2.path().join("vi");
        let emacs = tmp3.path().join("emacs");
        File::create(&nano).expect("should create file");
        File::create(&vim).expect("should create file");
        File::create(&vi).expect("should create file");
        File::create(&emacs).expect("should create file");

        assert_eq!(
            Edit::determine_editor_from_vars(visual_var, editor_var, path_var)
                .expect("should determine default editor"),
            (nano, Vec::<String>::new())
        );

        assert!(tmp1.path().is_dir());
        assert!(tmp2.path().is_dir());
        assert!(tmp3.path().is_dir());
    }

    /// Do not default to directories
    #[test]
    fn test_determine_editor_from_vars_no_directory() {
        let visual_var = "".to_owned();
        let editor_var = "".to_owned();

        let tmp1 = tempdir().expect("should create tempdir");
        let tmp2 = tempdir().expect("should create tempdir");
        let tmp3 = tempdir().expect("should create tempdir");

        let path_var = std::env::join_paths([&tmp1, &tmp2, &tmp3].map(|d| d.path().to_owned()))
            .expect("should path-join tmpdirs")
            .into_string()
            .expect("should convert paths from OsString to String");

        let nano = tmp1.path().join("nano");
        let vim = tmp2.path().join("vim");
        let vi = tmp2.path().join("vi");
        let emacs = tmp3.path().join("emacs");

        fs::create_dir(&nano).expect("should create directory");

        File::create(&vim).expect("should create file");
        File::create(&vi).expect("should create file");
        File::create(&emacs).expect("should create file");

        assert_eq!(
            Edit::determine_editor_from_vars(visual_var, editor_var, path_var)
                .expect("should determine default editor"),
            (vim, Vec::<String>::new())
        );

        assert!(tmp1.path().is_dir());
        assert!(tmp2.path().is_dir());
        assert!(tmp3.path().is_dir());
    }

    /// Return VISUAL before EDITOR, do not default to PATH
    #[test]
    fn test_determine_editor_from_vars_visual() {
        let visual_var = "micro".to_owned();
        let editor_var = "hx".to_owned();

        let tmp1 = tempdir().expect("should create tempdir");
        let tmp2 = tempdir().expect("should create tempdir");
        let tmp3 = tempdir().expect("should create tempdir");

        let path_var = std::env::join_paths([&tmp1, &tmp2, &tmp3].map(|d| d.path().to_owned()))
            .expect("should path-join tmpdirs")
            .into_string()
            .expect("should convert paths from OsString to String");

        let nano = tmp1.path().join("nano");
        let vim = tmp2.path().join("vim");
        let vi = tmp2.path().join("vi");
        let emacs = tmp3.path().join("emacs");
        File::create(&nano).expect("should create file");
        File::create(&vim).expect("should create file");
        File::create(&vi).expect("should create file");
        File::create(&emacs).expect("should create file");

        assert_eq!(
            Edit::determine_editor_from_vars(visual_var, editor_var, path_var)
                .expect("should determine default editor"),
            (PathBuf::from("micro"), Vec::<String>::new())
        );

        assert!(tmp1.path().is_dir());
        assert!(tmp2.path().is_dir());
        assert!(tmp3.path().is_dir());
    }

    /// Fallback to EDITOR, no default editor available in PATH
    #[test]
    fn test_determine_editor_from_vars_editor() {
        let visual_var = "".to_owned();
        let editor_var = "hx".to_owned();

        let tmp1 = tempdir().expect("should create tempdir");
        let tmp2 = tempdir().expect("should create tempdir");
        let tmp3 = tempdir().expect("should create tempdir");

        let path_var = std::env::join_paths([&tmp1, &tmp2, &tmp3].map(|d| d.path().to_owned()))
            .expect("should path-join tmpdirs")
            .into_string()
            .expect("should convert paths from OsString to String");

        assert_eq!(
            Edit::determine_editor_from_vars(visual_var, editor_var, path_var)
                .expect("should determine default editor"),
            (PathBuf::from("hx"), Vec::<String>::new())
        );

        assert!(tmp1.path().is_dir());
        assert!(tmp2.path().is_dir());
        assert!(tmp3.path().is_dir());
    }

    /// Split VISUAL into editor and args
    #[test]
    fn test_determine_editor_from_vars_visual_with_args() {
        let visual_var = "  code -w --reuse-window   --userdata-dir /home/user/code  ".to_owned();
        let editor_var = "hx".to_owned();

        let path_var = "".to_owned();

        assert_eq!(
            Edit::determine_editor_from_vars(visual_var, editor_var, path_var)
                .expect("should determine default editor"),
            (
                PathBuf::from("code"),
                vec!["-w", "--reuse-window", "--userdata-dir", "/home/user/code"]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
    }

    /// Split EDITOR into editor and args
    #[test]
    fn test_determine_editor_from_vars_editor_with_args() {
        let visual_var = "".to_owned();
        let editor_var = "code -w".to_owned();

        let path_var = "".to_owned();

        assert_eq!(
            Edit::determine_editor_from_vars(visual_var, editor_var, path_var)
                .expect("should determine default editor"),
            (
                PathBuf::from("code"),
                vec!["-w"].into_iter().map(String::from).collect()
            )
        );
    }

    /// VISUAL whitespace only defaults to EDITOR before PATH
    #[test]
    fn test_determine_editor_from_vars_visual_whitespace() {
        let visual_var = "       ".to_owned();
        let editor_var = "code -w".to_owned();

        let tmp1 = tempdir().expect("should create tempdir");
        let tmp2 = tempdir().expect("should create tempdir");
        let tmp3 = tempdir().expect("should create tempdir");

        let path_var = std::env::join_paths([&tmp1, &tmp2, &tmp3].map(|d| d.path().to_owned()))
            .expect("should path-join tmpdirs")
            .into_string()
            .expect("should convert paths from OsString to String");

        let nano = tmp1.path().join("nano");
        let vim = tmp2.path().join("vim");
        File::create(&nano).expect("should create file");
        File::create(&vim).expect("should create file");

        assert_eq!(
            Edit::determine_editor_from_vars(visual_var, editor_var, path_var)
                .expect("should determine default editor"),
            (
                PathBuf::from("code"),
                vec!["-w"].into_iter().map(String::from).collect()
            )
        );

        assert!(tmp1.path().is_dir());
        assert!(tmp2.path().is_dir());
        assert!(tmp3.path().is_dir());
    }

    /// EDITOR whitespace only defaults to editors on PATH
    #[test]
    fn test_determine_editor_from_vars_editor_whitespace() {
        let visual_var = "".to_owned();
        let editor_var = "       ".to_owned();

        let tmp1 = tempdir().expect("should create tempdir");
        let tmp2 = tempdir().expect("should create tempdir");
        let tmp3 = tempdir().expect("should create tempdir");

        let path_var = std::env::join_paths([&tmp1, &tmp2, &tmp3].map(|d| d.path().to_owned()))
            .expect("should path-join tmpdirs")
            .into_string()
            .expect("should convert paths from OsString to String");

        let nano = tmp1.path().join("nano");
        let vim = tmp2.path().join("vim");
        File::create(&nano).expect("should create file");
        File::create(&vim).expect("should create file");

        assert_eq!(
            Edit::determine_editor_from_vars(visual_var, editor_var, path_var)
                .expect("should determine default editor"),
            (nano, Vec::new())
        );

        assert!(tmp1.path().is_dir());
        assert!(tmp2.path().is_dir());
        assert!(tmp3.path().is_dir());
    }

    /// VISUAL and EDITOR whitespace only defaults to editors on PATH
    #[test]
    fn test_determine_editor_from_vars_whitespace() {
        let visual_var = "       ".to_owned();
        let editor_var = "       ".to_owned();

        let tmp1 = tempdir().expect("should create tempdir");
        let tmp2 = tempdir().expect("should create tempdir");
        let tmp3 = tempdir().expect("should create tempdir");

        let path_var = std::env::join_paths([&tmp1, &tmp2, &tmp3].map(|d| d.path().to_owned()))
            .expect("should path-join tmpdirs")
            .into_string()
            .expect("should convert paths from OsString to String");

        let nano = tmp1.path().join("nano");
        let vim = tmp2.path().join("vim");
        File::create(&nano).expect("should create file");
        File::create(&vim).expect("should create file");

        assert_eq!(
            Edit::determine_editor_from_vars(visual_var, editor_var, path_var)
                .expect("should determine default editor"),
            (nano, Vec::<String>::new())
        );

        assert!(tmp1.path().is_dir());
        assert!(tmp2.path().is_dir());
        assert!(tmp3.path().is_dir());
    }

    /// Given a v0 manifest that can be migrated and v0 contents, the migration
    /// should succeed,
    /// but the edit should fail.
    #[tokio::test]
    async fn migration_successful_migration_unsuccessful_edit() {
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub_and_client(None, true);
        let mut concrete_environment = ConcreteEnvironment::Path(new_path_environment(&flox, ""));
        let new_contents = indoc! {r#"
            [options]
            allow.broken = false
            "#};

        let err = Edit::edit_manifest(
            &flox,
            &mut concrete_environment,
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

        let actual_contents = concrete_environment
            .into_dyn_environment()
            .manifest_contents(&flox)
            .unwrap();
        assert_eq!(actual_contents, "version = 1\n");
    }

    /// Given a v0 manifest that cannot be migrated and v0 contents, the migration
    /// should fail,
    /// and the edit should fail.
    #[tokio::test]
    async fn migration_unsuccessful_migration_unsuccessful_edit() {
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub_and_client(None, true);

        let mut concrete_environment =
            ConcreteEnvironment::Path(new_path_environment(&flox, MANIFEST_V0_FIELDS));

        let new_contents = indoc! {r#"
            [options]
            allow.broken = false
            "#};

        let err = Edit::edit_manifest(
            &flox,
            &mut concrete_environment,
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

        let actual_contents = concrete_environment
            .into_dyn_environment()
            .manifest_contents(&flox)
            .unwrap();
        assert!(!actual_contents.contains("version = 1"));
    }

    /// Given a v0 manifest that cannot be migrated and v1 contents, the migration
    /// should fail,
    /// but the edit should succeed.
    #[tokio::test]
    async fn migration_unsuccessful_migration_successful_edit() {
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub_and_client(None, true);

        let mut concrete_environment =
            ConcreteEnvironment::Path(new_path_environment(&flox, MANIFEST_V0_FIELDS));

        let new_contents = indoc! {r#"
            version = 1

            [options]
            allow.broken = false
            "#};

        Edit::edit_manifest(
            &flox,
            &mut concrete_environment,
            Some(new_contents.to_string()),
        )
        .await
        .unwrap();

        // TODO: would be nice to make an assertion about
        // `Failed to migrate environment to version 1` being printed.

        let actual_contents = concrete_environment
            .dyn_environment_ref_mut()
            .manifest_contents(&flox)
            .unwrap();
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

        let mut concrete_environment =
            ConcreteEnvironment::Path(new_path_environment(&flox, old_contents));

        let new_contents = indoc! {r#"
            version = 1

            [options]
            allow.broken = false
            "#};

        Edit::edit_manifest(
            &flox,
            &mut concrete_environment,
            Some(new_contents.to_string()),
        )
        .await
        .unwrap();

        let actual_contents = concrete_environment
            .into_dyn_environment()
            .manifest_contents(&flox)
            .unwrap();
        assert!(actual_contents.contains("version = 1"));
    }

    /// If no no manifest file or contents are provided,
    /// edits should be blocked if the local checkout is out of sync.
    #[tokio::test]
    async fn edit_requires_sync_checkout() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) =
            flox_instance_with_optional_floxhub_and_client(Some(&owner), true);
        let old_contents = indoc! {r#"
            version = 1
        "#};

        let new_contents = indoc! {r#"
            version = 1

            [vars]
            foo = "bar"
        "#};

        let environment = mock_managed_environment(&flox, old_contents, owner);

        // edit the local manifest
        fs::write(environment.manifest_path(&flox).unwrap(), new_contents).unwrap();

        let err = Edit::edit_manifest(&flox, &mut ConcreteEnvironment::Managed(environment), None)
            .await
            .expect_err("edit should fail");

        let err = err
            .downcast::<ManagedEnvironmentError>()
            .expect("should be a ManagedEnvironmentError");

        assert!(matches!(err, ManagedEnvironmentError::CheckoutOutOfSync));
    }

    /// If a manifest file or contents are provided, edit succeeds despite local changes.
    #[tokio::test]
    async fn edit_with_file_ignores_local_changes() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) =
            flox_instance_with_optional_floxhub_and_client(Some(&owner), true);
        let old_contents = indoc! {r#"
            version = 1
        "#};

        let new_contents = indoc! {r#"
            version = 1

            [vars]
            foo = "bar"
        "#};

        let environment = mock_managed_environment(&flox, old_contents, owner);

        // edit the local manifest
        fs::write(environment.manifest_path(&flox).unwrap(), new_contents).unwrap();

        Edit::edit_manifest(
            &flox,
            &mut ConcreteEnvironment::Managed(environment),
            Some(new_contents.to_string()),
        )
        .await
        .expect("edit should succeed");
    }
}
