use std::fmt::Display;

use anyhow::Result;
use bpaf::Bpaf;
use crossterm::style::Stylize;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::generations::{
    self,
    GenerationsEnvironment,
    GenerationsExt,
    HistorySpec,
};
use indoc::formatdoc;
use tracing::instrument;

use crate::commands::{EnvironmentSelect, environment_select};
use crate::environment_subcommand_metric;
use crate::utils::message::{page_output, stdout_supports_color};

/// Arguments for the `flox generations history` command
#[derive(Bpaf, Debug, Clone)]
pub struct History {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(external(output_mode))]
    output_mode: OutputMode,

    /// Disable interactive pager
    #[bpaf(long)]
    no_pager: bool,
}

#[derive(Bpaf, Debug, Clone, PartialEq)]
#[bpaf(fallback(OutputMode::Pretty))]
enum OutputMode {
    #[bpaf(skip)]
    Pretty,
    /// Render generations as json
    #[bpaf(long)]
    Json,
}

impl History {
    #[instrument(name = "history", skip_all)]
    pub fn handle(self, flox: Flox) -> Result<()> {
        let env = self
            .environment
            .detect_concrete_environment(&flox, "Show history for")?;
        environment_subcommand_metric!("generations::history", env);

        let env: GenerationsEnvironment = env.try_into()?;
        let metadata = env.generations_metadata()?;

        let output = match self.output_mode {
            OutputMode::Pretty => DisplayHistory {
                history: metadata.history(),
                pretty: stdout_supports_color(),
            }
            .to_string(),
            OutputMode::Json => {
                serde_json::to_string_pretty(metadata.history()).expect("derived from valid json")
            },
        };

        if self.no_pager {
            println!("{output}");
            return Ok(());
        }

        page_output(output)
    }
}

/// Formatter container for [SingleGenerationMetadata].
/// Implements CLI/command specific formatting.
struct DisplayChange<'m> {
    change: &'m HistorySpec,
    // Whether to use pretty formatting (bold text)
    pretty: bool,
}
impl Display for DisplayChange<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let date = self.change.timestamp;
        let author = &self.change.author;
        let host = &self.change.hostname;
        let summary = self.change.summary();
        let generation = self.change.current_generation;

        let line = format!("Date:       {date}");
        if self.pretty {
            writeln!(f, "{}", line.bold())?;
        } else {
            writeln!(f, "{}", line)?;
        }
        write!(f, "{}", formatdoc! {"
            Author:     {author}
            Host:       {host}
            Generation: {generation}
            "})?;

        if let Some(command) = &self.change.command {
            let command = command.join(" ");
            write!(f, "{}", formatdoc! {"
            Command:    {command}
            "})?;
        }
        write!(f, "{}", formatdoc! {"
        Summary:    {summary}"})
    }
}

/// Formatter container for [AllGenerationsMetadata].
/// List formatting of generation data, following the template
///
/// ```text
/// <generation metadata>          # implemented by [DisplayMetadata] above
///
/// ...
/// ```
struct DisplayHistory<'m> {
    history: &'m generations::History,
    pretty: bool,
}
impl Display for DisplayHistory<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.history.into_iter().rev().peekable();
        while let (Some(change), peek) = (iter.next(), iter.peek()) {
            let next = DisplayChange {
                change,
                pretty: self.pretty,
            };
            write!(f, "{}", next)?;
            if peek.is_some() {
                writeln!(f)?;
                writeln!(f)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration};
    use flox_rust_sdk::models::environment::generations::test_helpers::{
        default_add_generation_options,
        default_switch_generation_options,
    };
    use flox_rust_sdk::models::environment::generations::{
        AddGenerationOptions,
        AllGenerationsMetadata,
        SwitchGenerationOptions,
    };
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_fmt_generations() {
        let mut metadata = AllGenerationsMetadata::default();
        metadata.add_generation(AddGenerationOptions {
            timestamp: DateTime::default() + Duration::hours(1),
            ..default_add_generation_options()
        });
        let (id, ..) = metadata.add_generation(AddGenerationOptions {
            timestamp: DateTime::default() + Duration::hours(2),
            ..default_add_generation_options()
        });
        metadata.add_generation(AddGenerationOptions {
            timestamp: DateTime::default() + Duration::hours(3),
            ..default_add_generation_options()
        });
        metadata
            .switch_generation(SwitchGenerationOptions {
                timestamp: DateTime::default() + Duration::hours(4),
                ..default_switch_generation_options(id)
            })
            .unwrap();

        let actual = DisplayHistory {
            history: metadata.history(),
            pretty: false,
        }
        .to_string();

        let expected = indoc! {"
            Date:       1970-01-01 04:00:00 UTC
            Author:     author
            Host:       host
            Generation: 2
            Command:    flox subcommand
            Summary:    changed current generation 3 -> 2

            Date:       1970-01-01 03:00:00 UTC
            Author:     author
            Host:       host
            Generation: 3
            Command:    flox subcommand
            Summary:    mock

            Date:       1970-01-01 02:00:00 UTC
            Author:     author
            Host:       host
            Generation: 2
            Command:    flox subcommand
            Summary:    mock

            Date:       1970-01-01 01:00:00 UTC
            Author:     author
            Host:       host
            Generation: 1
            Command:    flox subcommand
            Summary:    mock"
        };

        assert_eq!(actual, expected);
    }
}
