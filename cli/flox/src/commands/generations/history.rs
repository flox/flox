use std::fmt::Display;

use anyhow::Result;
use bpaf::Bpaf;
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

/// Arguments for the `flox generations history` command
#[derive(Bpaf, Debug, Clone)]
pub struct History {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
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

        println!("{}", DisplayHistory(metadata.history()));
        Ok(())
    }
}

/// Formatter container for [SingleGenerationMetadata].
/// Implements CLI/command specific formatting.
struct DisplayChange<'m> {
    change: &'m HistorySpec,
}
impl Display for DisplayChange<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let date = self.change.timestamp;
        let author = &self.change.author;
        let host = &self.change.hostname;
        let summary = self.change.summary();
        let generation = self.change.current_generation;

        write!(f, "{}", formatdoc! {"
            Date:       {date}
            Author:     {author}
            Host:       {host}
            Generation: {generation}
            Summary:    {summary}"})
    }
}

/// Formatter container for [AllGenerationsMetadata].
/// List formatting of generation data, following the template
///
/// ```text
/// * <generation id>[ (current)]:
///   <generation metadata>          # implemented by [DisplayMetadata] above
/// ```
struct DisplayHistory<'m>(&'m generations::History);
impl Display for DisplayHistory<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.0.into_iter().peekable();
        while let (Some(change), peek) = (iter.next(), iter.peek()) {
            let next = DisplayChange { change };
            write!(f, "* {}", indent::indent_by(2, next.to_string()))?;
            if peek.is_some() {
                writeln!(f)?;
                writeln!(f)?;
            }
        }
        Ok(())
    }
}
