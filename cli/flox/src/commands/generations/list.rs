use std::fmt::Display;

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::generations::{
    AllGenerationsMetadata,
    GenerationsEnvironment,
    GenerationsExt,
    SingleGenerationMetadata,
};
use tracing::instrument;

use crate::commands::{EnvironmentSelect, environment_select};
use crate::environment_subcommand_metric;

/// Arguments for the `flox generations list` command
#[derive(Bpaf, Debug, Clone)]
pub struct List {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl List {
    #[instrument(name = "list", skip_all)]
    pub fn handle(self, flox: Flox) -> Result<()> {
        let env = self.environment.to_concrete_environment(&flox)?;
        environment_subcommand_metric!("generations::list", env);

        let env: GenerationsEnvironment = env.try_into()?;
        let metadata = env.generations_metadata()?;

        println!("{}", DisplayAllMetadata(&metadata));
        Ok(())
    }
}

/// Formatter container for [SingleGenerationMetadata].
/// Implements CLI/command specific formatting.
struct DisplayMetadata<'m> {
    metadata: &'m SingleGenerationMetadata,
}
impl Display for DisplayMetadata<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Description: {}", self.metadata.description)?;
        write!(f, "Created: {}", self.metadata.created)?;
        if let Some(last_active) = self.metadata.last_active {
            writeln!(f)?;
            write!(f, "Last Active: {last_active}")?;
        };
        Ok(())
    }
}

/// Formatter container for [AllGenerationsMetadata].
/// List formatting of generation data, following the template
///
/// ```text
/// * <generation id>[ (current)]:
///   <generation metadata>          # implemented by [DisplayMetadata] above
/// ```
struct DisplayAllMetadata<'m>(&'m AllGenerationsMetadata);
impl Display for DisplayAllMetadata<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.0.generations().into_iter().rev().peekable();
        while let (Some((id, metadata)), peek) = (iter.next(), iter.peek()) {
            write!(f, "* {id}")?;
            if Some(id) == self.0.current_gen() {
                write!(f, " (current)")?;
            }
            writeln!(f, ":")?;

            let next = DisplayMetadata {
                metadata: &metadata,
            };
            write!(f, "{}", indent::indent_all_by(2, next.to_string()))?;
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
        SingleGenerationMetadata,
        SwitchGenerationOptions,
    };
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_fmt_single_generation() {
        let actual = DisplayMetadata {
            metadata: &SingleGenerationMetadata {
                created: DateTime::default(),
                last_active: Some(DateTime::default()),
                description: "Generation description".to_string(),
            },
        }
        .to_string();

        let expected = indoc! {"
            Description: Generation description
            Created: 1970-01-01 00:00:00 UTC
            Last Active: 1970-01-01 00:00:00 UTC"
        };

        assert_eq!(actual, expected);
    }

    /// Currently prevented by the implementation
    #[test]
    fn test_fmt_single_generation_never_active() {
        let actual = DisplayMetadata {
            metadata: &SingleGenerationMetadata {
                created: DateTime::default(),
                last_active: None,
                description: "Generation description".to_string(),
            },
        }
        .to_string();

        let expected = indoc! {"
            Description: Generation description
            Created: 1970-01-01 00:00:00 UTC"
        };

        assert_eq!(actual, expected);
    }

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

        let actual = DisplayAllMetadata(&metadata).to_string();

        let expected = indoc! {"
            * 3:
              Description: mock
              Created: 1970-01-01 03:00:00 UTC
              Last Active: 1970-01-01 04:00:00 UTC

            * 2 (current):
              Description: mock
              Created: 1970-01-01 02:00:00 UTC

            * 1:
              Description: mock
              Created: 1970-01-01 01:00:00 UTC
              Last Active: 1970-01-01 02:00:00 UTC"
        };

        assert_eq!(actual, expected);
    }
}
