use std::num::NonZeroU64;

use anyhow::Result;
use bpaf::Bpaf;
use floxhub_client::{BuildResponse, FactoryClientTrait};
use indoc::formatdoc;
use serde::Serialize;
use tracing::instrument;

use super::effective_status;
use crate::subcommand_metric;

/// Show the status of a single Flox Factory build.
#[derive(Debug, Clone, PartialEq, Bpaf)]
pub struct Status {
    /// Display output as JSON
    #[bpaf(long)]
    pub json: bool,

    /// Build ID to query
    #[bpaf(positional("ID"))]
    pub id: NonZeroU64,
}

impl Status {
    #[instrument(name = "status", skip_all)]
    pub async fn handle(self, client: &impl FactoryClientTrait) -> Result<()> {
        subcommand_metric!("factory::status");

        let build = client.get_build(self.id.get() as i64).await.map_err(|e| {
            super::user_facing_error(
                e,
                Some(formatdoc! {"
                    No Flox Factory build found with ID {id}.
                    Use 'flox factory list' to see existing builds.",
                    id = self.id,
                }),
            )
        })?;

        print!("{}", render(build, self.json)?);
        Ok(())
    }
}

/// Render a single build as either raw JSON or a human-readable table.
///
/// The JSON form is intentionally the bare [`BuildResponse`] object, with no
/// surrounding envelope: a single build has no pagination to report, unlike the
/// `list` verb whose JSON form carries the page envelope.
fn render(build: BuildResponse, json: bool) -> Result<String> {
    if json {
        Ok(format!("{}\n", serde_json::to_string_pretty(&build)?))
    } else {
        Ok(BuildStatusDisplay::from(build).to_string())
    }
}

/// Human-readable single-build status table.
#[derive(Clone, Debug, Serialize)]
struct BuildStatusDisplay {
    build_id: i64,
    system: String,
    attr_path: String,
    catalog_name: String,
    status: String,
    created_at: String,
}

impl From<BuildResponse> for BuildStatusDisplay {
    fn from(b: BuildResponse) -> Self {
        let status = effective_status(&b);

        BuildStatusDisplay {
            build_id: b.build_id,
            system: b.system,
            attr_path: b.attr_path,
            catalog_name: b.catalog_name,
            status,
            created_at: b.created_at.to_rfc3339(),
        }
    }
}

impl std::fmt::Display for BuildStatusDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{:<14} {}", "Build ID:", self.build_id)?;
        writeln!(f, "{:<14} {}", "System:", self.system)?;
        writeln!(f, "{:<14} {}", "Attr path:", self.attr_path)?;
        writeln!(f, "{:<14} {}", "Catalog:", self.catalog_name)?;
        writeln!(f, "{:<14} {}", "Status:", self.status)?;
        writeln!(f, "{:<14} {}", "Created:", self.created_at)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::commands::factory::test_helpers::{StubFactoryClient, make_build};

    #[tokio::test]
    async fn status_renders_not_found_message() {
        let client = StubFactoryClient::with_not_found();
        let args = Status {
            id: NonZeroU64::new(42).unwrap(),
            json: false,
        };

        let err = args.handle(&client).await.unwrap_err();
        assert_eq!(err.to_string(), indoc! {"
            No Flox Factory build found with ID 42.
            Use 'flox factory list' to see existing builds."});
    }

    #[test]
    fn status_table_uses_task_status_when_dispatched() {
        let build = make_build(42, "x86_64-linux", "hello", Some("running"));
        assert_eq!(render(build, false).unwrap(), indoc! {"
            Build ID:      42
            System:        x86_64-linux
            Attr path:     hello
            Catalog:       my-catalog
            Status:        running
            Created:       2025-01-01T00:00:00+00:00
        "});
    }

    #[test]
    fn status_table_marks_undispatched_build_distinctly() {
        let build = make_build(42, "x86_64-linux", "hello", None);
        assert_eq!(render(build, false).unwrap(), indoc! {"
            Build ID:      42
            System:        x86_64-linux
            Attr path:     hello
            Catalog:       my-catalog
            Status:        pending (not dispatched)
            Created:       2025-01-01T00:00:00+00:00
        "});
    }
}
