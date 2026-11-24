use anyhow::Result;
use bpaf::Bpaf;
use floxhub_client::{FactoryClientTrait, FactoryRunDetailResponse, FactoryRunId};
use indoc::formatdoc;
use tracing::instrument;

use super::{effective_status, trigger_label};
use crate::subcommand_metric;

/// Show the status of a single Flox Factory run.
#[derive(Debug, Clone, PartialEq, Bpaf)]
pub struct Status {
    /// Display output as JSON
    #[bpaf(long)]
    pub json: bool,

    /// ID of the Flox Factory run to query
    #[bpaf(positional("ID"))]
    pub id: FactoryRunId,
}

impl Status {
    #[instrument(name = "status", skip_all)]
    pub async fn handle(self, client: &impl FactoryClientTrait) -> Result<()> {
        subcommand_metric!("factory::status");

        let not_found = formatdoc! {"
            No Flox Factory run found with ID {id}.
            Use 'flox factory list' to see existing runs.",
            id = self.id,
        };

        let run = client
            .get_factory_run(self.id)
            .await
            .map_err(|e| super::user_facing_error(e, Some(not_found)))?;

        print!("{}", render(run, self.json)?);
        Ok(())
    }
}

/// Render a single run as either raw JSON or a human-readable table.
///
/// The JSON form is intentionally the bare [`FactoryRunDetailResponse`] object, with
/// no surrounding envelope: a single run has no pagination to report.
fn render(run: FactoryRunDetailResponse, json: bool) -> Result<String> {
    if json {
        Ok(format!("{}\n", serde_json::to_string_pretty(&run)?))
    } else {
        Ok(RunStatusDisplay::from(run).to_string())
    }
}

/// Human-readable single-run status table.
#[derive(Clone, Debug)]
struct RunStatusDisplay {
    run_id: i64,
    pkg_path: String,
    system: String,
    trigger: String,
    status: String,
    created_at: String,
}

impl From<FactoryRunDetailResponse> for RunStatusDisplay {
    fn from(run: FactoryRunDetailResponse) -> Self {
        let status = effective_status(&run.status);
        let trigger = trigger_label(run.trigger.as_ref());

        RunStatusDisplay {
            run_id: run.run_id,
            pkg_path: format!("{}/{}", run.catalog_name, run.attr_path),
            system: run.system,
            trigger,
            status,
            created_at: run.created_at.to_rfc3339(),
        }
    }
}

impl std::fmt::Display for RunStatusDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{:<14} {}", "Run ID:", self.run_id)?;
        writeln!(f, "{:<14} {}", "Package:", self.pkg_path)?;
        writeln!(f, "{:<14} {}", "System:", self.system)?;
        writeln!(f, "{:<14} {}", "Trigger:", self.trigger)?;
        writeln!(f, "{:<14} {}", "Status:", self.status)?;
        writeln!(f, "{:<14} {}", "Created:", self.created_at)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use floxhub_client::{EffectiveFactoryRunStatus, FactoryRunTrigger};
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::commands::factory::test_helpers::{StubFactoryClient, make_run, make_run_detail};

    #[tokio::test]
    async fn status_renders_not_found_message() {
        let client = StubFactoryClient::with_not_found();
        let args = Status {
            id: "42".parse().unwrap(),
            json: false,
        };

        let err = args.handle(&client).await.unwrap_err();
        assert_eq!(err.to_string(), indoc! {"
            No Flox Factory run found with ID 42.
            Use 'flox factory list' to see existing runs."});
    }

    #[test]
    fn status_table_shows_effective_status_when_dispatched() {
        let mut run = make_run(
            42,
            "x86_64-linux",
            "hello",
            Some(EffectiveFactoryRunStatus::Running),
        );
        run.trigger = Some(FactoryRunTrigger {
            kind: Some("source_push".to_string().into()),
            stabilities: vec!["staging".to_string()],
        });
        assert_eq!(render(make_run_detail(run), false).unwrap(), indoc! {"
            Run ID:        42
            Package:       my-catalog/hello
            System:        x86_64-linux
            Trigger:       source_push (staging)
            Status:        running
            Created:       2025-01-01T00:00:00+00:00
        "});
    }

    #[test]
    fn status_table_shows_pending_before_dispatch() {
        let run = make_run_detail(make_run(42, "x86_64-linux", "hello", None));
        assert_eq!(render(run, false).unwrap(), indoc! {"
            Run ID:        42
            Package:       my-catalog/hello
            System:        x86_64-linux
            Trigger:       -
            Status:        pending
            Created:       2025-01-01T00:00:00+00:00
        "});
    }
}
