use std::env;
use std::fs::File;
use std::io::stdin;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::{EnvironmentName, Flox};
use flox_rust_sdk::models::environment::{
    CoreEnvironmentError,
    EditResult,
    Environment,
    EnvironmentError,
};
use itertools::Itertools;
use log::debug;
use source_span::fmt::{Formatter, Style};
use source_span::{Position, Span};
use tracing::instrument;

use super::{
    activated_environments,
    environment_select,
    EnvironmentSelect,
    UninitializedEnvironment,
};
use crate::commands::{ensure_floxhub_token, ConcreteEnvironment};
use crate::subcommand_metric;
use crate::utils::dialog::{Confirm, Dialog, Spinner};
use crate::utils::errors::{
    apply_doc_link_for_unsupported_packages,
    format_core_error,
    format_locked_manifest_error,
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
}

impl Edit {
    #[instrument(name = "edit", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        subcommand_metric!("edit");

        let detected_environment = self
            .environment
            .detect_concrete_environment(&flox, "Edit")?;

        // Ensure the user is logged in for the following remote operations
        if let ConcreteEnvironment::Remote(_) = detected_environment {
            ensure_floxhub_token(&mut flox).await?;
        };

        match self.action {
            EditAction::EditManifest { file } => {
                // TODO: differentiate between interactive edits and replacement
                let span = tracing::info_span!("edit_file");
                let _guard = span.enter();
                Self::edit_manifest(&flox, detected_environment, file).await?
            },
            EditAction::Rename { name } => {
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
        }

        Ok(())
    }

    async fn edit_manifest(
        flox: &Flox,
        detected_environment: ConcreteEnvironment,
        file: Option<PathBuf>,
    ) -> Result<()> {
        let active_environment =
            UninitializedEnvironment::from_concrete_environment(&detected_environment)?;
        let mut environment = detected_environment.into_dyn_environment();

        let result = match Self::provided_manifest_contents(file)? {
            // If provided with the contents of a manifest file, either via a path to a file or via
            // contents piped to stdin, use those contents to try building the environment.
            Some(new_manifest) => environment
                .edit(flox, new_manifest)
                .map_err(apply_doc_link_for_unsupported_packages)?,
            // If not provided with new manifest contents, let the user edit the file directly
            // via $EDITOR or $VISUAL (as long as `flox edit` was invoked interactively).
            None => Self::interactive_edit(flox, environment.as_mut()).await?,
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
        let should_continue = Dialog {
            message: "Continue editing?",
            help_message: Default::default(),
            typed: Confirm {
                default: Some(true),
            },
        };

        // Let the user keep editing the file until the build succeeds or the user
        // decides to stop.
        loop {
            tracing::debug!("inside the edit loop");
            let new_manifest = Edit::edited_manifest_contents(&tmp_manifest, &editor)?;

            let result = Dialog {
                message: "Building environment to validate edit...",
                help_message: None,
                typed: Spinner::new(|| environment.edit(flox, new_manifest.clone())),
            }
            .spin()
            .map_err(apply_doc_link_for_unsupported_packages);

            match result {
                Err(EnvironmentError::Core(CoreEnvironmentError::LockedManifest(e))) => {
                    message::error(format_locked_manifest_error(&e));

                    if !Dialog::can_prompt() {
                        bail!("Can't prompt to continue editing in non-interactive context");
                    }
                    if !should_continue.clone().prompt().await? {
                        bail!("Environment editing cancelled");
                    }
                },
                Err(EnvironmentError::Core(
                    ref e @ CoreEnvironmentError::DeserializeManifest(ref err),
                )) => {
                    tracing::debug!("formatting manifest error");
                    let maybe_byte_span = err.span();
                    if let Some(byte_span) = maybe_byte_span {
                        tracing::debug!("got byte span");
                        // Confusingly:
                        // - start = first character of span
                        // - last = last character of span
                        // - end = first character after span
                        let (start_line, start_col) =
                            Self::translate_position(&new_manifest, byte_span.start);
                        let (last_line, last_col) =
                            Self::translate_position(&new_manifest, byte_span.end);
                        let (end_line, end_col) = Self::translate_position(
                            &new_manifest,
                            new_manifest.len().min(byte_span.end + 1),
                        );
                        let manifest_span = Span::new(
                            Position::new(start_line, start_col),
                            Position::new(last_line, last_col),
                            Position::new(end_line, end_col),
                        );
                        let mut formatter = Formatter::new();
                        formatter.add(manifest_span, Some(err.to_string()), Style::Error);
                        let char_metrics = source_span::DefaultMetrics::with_tab_stop(4);
                        let formatted = formatter
                            .render(
                                new_manifest.chars().map(Ok::<char, anyhow::Error>),
                                manifest_span,
                                &char_metrics,
                            )
                            .unwrap();
                        message::error(formatted);
                        if !Dialog::can_prompt() {
                            bail!("Can't prompt to continue editing in non-interactive context");
                        }
                        if !should_continue.clone().prompt().await? {
                            bail!("Environment editing cancelled");
                        }
                    } else {
                        message::error(format_core_error(e));

                        if !Dialog::can_prompt() {
                            bail!("Can't prompt to continue editing in non-interactive context");
                        }
                        if !should_continue.clone().prompt().await? {
                            bail!("Environment editing cancelled");
                        }
                    }
                },
                Err(e) => {
                    bail!(e)
                },
                Ok(result) => {
                    return Ok(result);
                },
            }
        }
    }

    // Shamelessly copied from cargo source code
    fn translate_position(input: &str, index: usize) -> (usize, usize) {
        if input.is_empty() {
            return (0, index);
        }

        let safe_index = index.min(input.len() - 1);
        let column_offset = index - safe_index;

        let nl = input[0..safe_index]
            .as_bytes()
            .iter()
            .rev()
            .enumerate()
            .find(|(_, b)| **b == b'\n')
            .map(|(nl, _)| safe_index - nl - 1);
        let line_start = match nl {
            Some(nl) => nl + 1,
            None => 0,
        };
        let line = input[0..line_start]
            .as_bytes()
            .iter()
            .filter(|c| **c == b'\n')
            .count();
        let column = input[line_start..=safe_index].chars().count() - 1;
        let column = column + column_offset;

        (line, column)
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
