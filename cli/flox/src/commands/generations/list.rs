use std::fmt::Display;

use anyhow::Result;
use bpaf::Bpaf;
use crossterm::style::Stylize;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::generations::{
    AllGenerationsMetadata,
    GenerationId,
    GenerationsEnvironment,
    GenerationsExt,
    SingleGenerationMetadata,
};
use indoc::formatdoc;
use renderdag::{Ancestor, GraphRowRenderer, Renderer as _};
use tracing::instrument;

use crate::commands::{EnvironmentSelect, environment_select};
use crate::environment_subcommand_metric;
use crate::utils::bail_on_v2_manifest_without_feature_flag;
use crate::utils::message::{page_output, stdout_supports_color};

/// Arguments for the `flox generations list` command
#[derive(Bpaf, Debug, Clone)]
pub struct List {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(long, short)]
    upstream: bool,

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
    /// Render generations as a tree
    #[bpaf(long, short)]
    Tree,
    /// Render generations as json
    #[bpaf(long)]
    Json,
}

impl List {
    #[instrument(name = "list", skip_all)]
    pub fn handle(self, flox: Flox) -> Result<()> {
        let env = self
            .environment
            .detect_concrete_environment(&flox, "List using")?;
        environment_subcommand_metric!(
            "generations::list",
            env,
            request_tree = self.output_mode == OutputMode::Tree
        );
        bail_on_v2_manifest_without_feature_flag(&flox, &env)?;

        let env: GenerationsEnvironment = env.try_into()?;
        let metadata = if self.upstream {
            env.remote_generations_metadata()?
        } else {
            env.generations_metadata()?
        };

        let output = match self.output_mode {
            OutputMode::Pretty => DisplayAllMetadata {
                metadata: &metadata,
                pretty: stdout_supports_color(),
            }
            .to_string(),
            OutputMode::Tree => render_tree(&metadata),
            OutputMode::Json => serde_json::to_string_pretty(&metadata.generations())
                .expect("derived from valid json"),
        };

        if self.no_pager {
            println!("{}", output);
            return Ok(());
        }

        page_output(output)
    }
}

/// Formatter container for [SingleGenerationMetadata].
/// Implements CLI/command specific formatting.
struct DisplayMetadata<'m> {
    metadata: &'m SingleGenerationMetadata,
}
impl Display for DisplayMetadata<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let description = &self.metadata.description;
        let created = self.metadata.created;
        let last_live = if let Some(last_live) = self.metadata.last_live {
            last_live.to_string()
        } else {
            "Now".to_string()
        };

        write!(f, "{}", formatdoc! {"
            Description: {description}
            Created:     {created}
            Last Live:   {last_live}"})
    }
}

/// Formatter container for [AllGenerationsMetadata].
/// List formatting of generation data, following the template
///
/// Current version:
/// ```text
/// Generation: <generation id> (live)
/// <generation metadata>          # implemented by [DisplayMetadata] above
/// ```
///
/// Other versions:
/// ```text
/// Generation: <generation id>
/// <generation metadata>
/// ```
struct DisplayAllMetadata<'m> {
    metadata: &'m AllGenerationsMetadata,
    // Whether to use pretty formatting (bold text and color)
    pretty: bool,
}
impl Display for DisplayAllMetadata<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut iter = self.metadata.generations().into_iter().rev().peekable();
        while let (Some((id, metadata)), peek) = (iter.next(), iter.peek()) {
            let generation = format!("Generation:  {id}");
            let current = if Some(id) == self.metadata.current_gen() {
                " (live)"
            } else {
                ""
            };
            if self.pretty {
                write!(f, "{}{}", generation.bold(), current.bold().yellow())?;
            } else {
                write!(f, "{}{}", generation, current)?;
            }
            writeln!(f)?;

            let next = DisplayMetadata {
                metadata: &metadata,
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

/// Render generations as a tree, clearly showing the parent generation,
/// and construction of the current state.
///
/// ## Example
///
/// ```text
/// 7  (live) installed package 'gum (gum)'
/// │  created:   2025-09-19 15:27:44 UTC
/// │  last live: Now
/// │ 6  installed package 'jq (jq)'
/// │ │  created:   2025-09-19 15:22:33 UTC
/// │ │  last live: 2025-09-19 15:27:18 UTC
/// │ │ 5  installed packages 'jq (jq)', 'htop (htop)'
/// │ │ │  created:   2025-09-19 15:21:57 UTC
/// │ │ │  last live: 2025-09-19 15:22:22 UTC
/// 4 │ │  installed package 'lolcat (lolcat)'
/// │ │ │  created:   2025-09-19 15:19:10 UTC
/// │ │ │  last live: 2025-09-19 15:27:44 UTC
/// │ 3 │  installed package 'htop (htop)'
/// ├─╯ │  created:   2025-09-18 15:53:59 UTC
/// │   │  last live: 2025-09-19 15:22:33 UTC
/// 2   │  installed package 'hello (hello)'
/// ├───╯  created:   2025-09-18 15:53:49 UTC
/// │      last live: 2025-09-19 15:19:10 UTC
/// 1  manually edited the manifest [metadata migrated]
/// │  created:   2025-08-07 16:02:02 UTC
/// ~  last live: 2025-09-19 15:21:57 UTC
/// ```
fn render_tree(metadata: &AllGenerationsMetadata) -> String {
    let mut graph_nodes: Vec<(GenerationId, SingleGenerationMetadata)> = Vec::new();

    for (id, generation) in metadata.generations() {
        graph_nodes.push((id, generation))
    }

    let current_gen = metadata.current_gen();

    let mut renderer = GraphRowRenderer::new()
        .output()
        .with_min_row_height(2)
        .build_box_drawing();

    graph_nodes
        .into_iter()
        .rev()
        .map(|(id, generation)| {
            let glyph = id.to_string();

            let live_prefix = if Some(id) == current_gen {
                "(live) "
            } else {
                ""
            };

            let description = format!("{live_prefix}{}", generation.description.bold());
            let created = generation.created;
            let last_live = if let Some(last_live) = generation.last_live {
                last_live.to_string()
            } else {
                "Now".to_string()
            };

            let message = formatdoc! {"
                {description}
                created:   {created}
                last live: {last_live}"};
            let parents = match generation.parent {
                None => vec![Ancestor::Anonymous],
                Some(parent) => vec![Ancestor::Parent(parent)],
            };

            renderer.next_row(id, parents, glyph, message)
        })
        .collect()
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
                parent: None,
                created: DateTime::default(),
                last_live: Some(DateTime::default()),
                description: "Generation description".to_string(),
            },
        }
        .to_string();

        let expected = indoc! {"
            Description: Generation description
            Created:     1970-01-01 00:00:00 UTC
            Last Live:   1970-01-01 00:00:00 UTC"
        };

        assert_eq!(actual, expected);
    }

    /// Currently prevented by the implementation
    #[test]
    fn test_fmt_single_generation_never_active() {
        let actual = DisplayMetadata {
            metadata: &SingleGenerationMetadata {
                parent: None,
                created: DateTime::default(),
                last_live: None,
                description: "Generation description".to_string(),
            },
        }
        .to_string();

        let expected = indoc! {"
            Description: Generation description
            Created:     1970-01-01 00:00:00 UTC
            Last Live:   Now"
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

        let actual = DisplayAllMetadata {
            metadata: &metadata,
            pretty: false,
        }
        .to_string();

        let expected = indoc! {"
            Generation:  3
            Description: mock
            Created:     1970-01-01 03:00:00 UTC
            Last Live:   1970-01-01 04:00:00 UTC

            Generation:  2 (live)
            Description: mock
            Created:     1970-01-01 02:00:00 UTC
            Last Live:   Now

            Generation:  1
            Description: mock
            Created:     1970-01-01 01:00:00 UTC
            Last Live:   1970-01-01 02:00:00 UTC"
        };

        assert_eq!(actual, expected);
    }
}
