use std::fmt;

use anyhow::Result;
use bpaf::Bpaf;
use floxhub_client::{BuildFilters, BuildResponse, EffectiveBuildStatus, FactoryClientTrait};
use itertools::Itertools;
use serde::Serialize;
use tracing::instrument;

use super::{effective_status, effective_updated_at};
use crate::subcommand_metric;
use crate::utils::message::page_output;

/// Parse one `--status` value into a typed [`EffectiveBuildStatus`], rejecting
/// any word outside the known vocabulary at the CLI boundary. The status
/// vocabulary is pinned by the vendored schema, so an invalid value is a
/// definite user error rather than something only the server can judge.
fn parse_status(s: String) -> Result<EffectiveBuildStatus, String> {
    EffectiveBuildStatus::KNOWN
        .iter()
        .find(|status| status.as_str() == s)
        .cloned()
        .ok_or_else(|| {
            let valid = EffectiveBuildStatus::KNOWN
                .iter()
                .map(EffectiveBuildStatus::as_str)
                .join(", ");
            format!("Invalid status '{s}'; valid values are: {valid}.")
        })
}

/// List Flox Factory builds.
///
/// Each filter is repeatable and ORs its values; different filters AND together.
/// An unfiltered invocation lists every build.
#[derive(Debug, Clone, PartialEq, Bpaf)]
pub struct List {
    /// Filter by build status; repeat to match any of several.
    /// Valid values: pending, running, completed, failed, timed_out,
    /// cancelled.
    #[bpaf(long, argument::<String>("STATUS"), parse(parse_status), many)]
    pub status: Vec<EffectiveBuildStatus>,

    /// Display output as JSON
    #[bpaf(long)]
    pub json: bool,

    /// Disable interactive pager
    #[bpaf(long)]
    pub no_pager: bool,
}

impl List {
    #[instrument(name = "list", skip_all)]
    pub async fn handle(self, client: &impl FactoryClientTrait) -> Result<()> {
        subcommand_metric!("factory::list");

        let filters = BuildFilters {
            status: self.status,
            ..Default::default()
        };

        // Depage the full result set, mirroring `flox generations list`: the
        // operator sees every matching build at once and scrolls with the
        // pager, rather than stepping server-side pages by hand.
        let builds = client
            .list_builds(&filters)
            .await
            .map_err(|e| super::user_facing_error(e, None))?;

        let output = render(builds.results, self.json)?;

        // JSON is for scripting: never route it through the pager, even on a
        // TTY. The human table is paged unless `--no-pager` is given.
        if self.json || self.no_pager {
            print!("{output}");
            return Ok(());
        }

        page_output(output)
    }
}

/// Render the builds as either pretty-printed JSON or a table.
///
/// The depaging client returns every matching build, so the JSON form is the
/// full array of builds, with no pagination envelope to report.
fn render(builds: Vec<BuildResponse>, json: bool) -> Result<String> {
    if json {
        Ok(format!("{}\n", serde_json::to_string_pretty(&builds)?))
    } else {
        Ok(BuildListDisplay::from(builds).to_string())
    }
}

/// Human-readable build list table row.
#[derive(Clone, Debug, Serialize)]
struct BuildRowDisplay {
    build_id: i64,
    attr_path: String,
    system: String,
    status: String,
    updated_at: String,
}

impl From<BuildResponse> for BuildRowDisplay {
    fn from(b: BuildResponse) -> Self {
        let status = effective_status(&b);
        let updated_at = effective_updated_at(&b);

        BuildRowDisplay {
            build_id: b.build_id,
            attr_path: b.attr_path,
            system: b.system,
            status,
            updated_at,
        }
    }
}

/// Human-readable build list table.
#[derive(Clone, Debug)]
struct BuildListDisplay {
    rows: Vec<BuildRowDisplay>,
}

impl From<Vec<BuildResponse>> for BuildListDisplay {
    fn from(builds: Vec<BuildResponse>) -> Self {
        BuildListDisplay {
            rows: builds.into_iter().map(BuildRowDisplay::from).collect(),
        }
    }
}

impl fmt::Display for BuildListDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.rows.is_empty() {
            writeln!(f, "No builds found.")?;
            return Ok(());
        }

        // Column widths with minimums sized to header labels.
        let id_width = "BUILD ID".len().max(
            self.rows
                .iter()
                .map(|r| r.build_id.to_string().len())
                .max()
                .unwrap_or(0),
        );
        let attr_width = "ATTR PATH".len().max(
            self.rows
                .iter()
                .map(|r| r.attr_path.len())
                .max()
                .unwrap_or(0),
        );
        let system_width = "SYSTEM"
            .len()
            .max(self.rows.iter().map(|r| r.system.len()).max().unwrap_or(0));
        let status_width = "STATUS"
            .len()
            .max(self.rows.iter().map(|r| r.status.len()).max().unwrap_or(0));

        writeln!(
            f,
            "{:<id_width$}  {:<attr_width$}  {:<system_width$}  {:<status_width$}  UPDATED",
            "BUILD ID", "ATTR PATH", "SYSTEM", "STATUS",
        )?;

        for row in &self.rows {
            writeln!(
                f,
                "{:<id_width$}  {:<attr_width$}  {:<system_width$}  {:<status_width$}  {}",
                row.build_id, row.attr_path, row.system, row.status, row.updated_at,
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bpaf::Parser;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::commands::factory::test_helpers::{StubFactoryClient, StubResult, make_build};

    #[test]
    fn list_display_renders_table_exactly() {
        // A dispatched build shows its task's updated_at; an undispatched build
        // has no task, so UPDATED falls back to the build's created_at.
        let builds = vec![
            make_build(
                1,
                "x86_64-linux",
                "hello",
                Some(EffectiveBuildStatus::Running),
            ),
            make_build(2, "aarch64-darwin", "ripgrep", None),
        ];
        let display = BuildListDisplay::from(builds);
        assert_eq!(display.to_string(), indoc! {"
            BUILD ID  ATTR PATH  SYSTEM          STATUS                    UPDATED
            1         hello      x86_64-linux    running                   2025-01-01T00:00:01+00:00
            2         ripgrep    aarch64-darwin  pending (not dispatched)  2025-01-01T00:00:00+00:00
        "});
    }

    #[test]
    fn list_display_renders_new_status_labels() {
        // The labels introduced with the typed status: a timed-out build reads
        // `timed_out` (not `failed`), a pre-dispatch cancel reads `cancelled`
        // (not undispatched), and an unrecognized status renders tolerantly as
        // `unknown: <value>` rather than blanking the row.
        let builds = vec![
            make_build(
                3,
                "x86_64-linux",
                "curl",
                Some(EffectiveBuildStatus::TimedOut),
            ),
            make_build(
                4,
                "aarch64-darwin",
                "jq",
                Some(EffectiveBuildStatus::Cancelled),
            ),
            make_build(
                5,
                "x86_64-linux",
                "wget",
                Some(EffectiveBuildStatus::Unknown("frobnicated".to_string())),
            ),
        ];
        let display = BuildListDisplay::from(builds);
        assert_eq!(display.to_string(), indoc! {"
            BUILD ID  ATTR PATH  SYSTEM          STATUS                UPDATED
            3         curl       x86_64-linux    timed_out             2025-01-01T00:00:01+00:00
            4         jq         aarch64-darwin  cancelled             2025-01-01T00:00:00+00:00
            5         wget       x86_64-linux    unknown: frobnicated  2025-01-01T00:00:00+00:00
        "});
    }

    #[tokio::test]
    async fn list_handler_forwards_status_filters() {
        let client = StubFactoryClient::with_outcomes(
            StubResult::Build(EffectiveBuildStatus::Completed),
            StubResult::NotFound,
        );
        let args = List {
            status: vec![EffectiveBuildStatus::Running, EffectiveBuildStatus::Failed],
            json: false,
            no_pager: true,
        };

        args.handle(&client).await.unwrap();

        assert_eq!(
            client.last_filters(),
            Some(BuildFilters {
                status: vec![EffectiveBuildStatus::Running, EffectiveBuildStatus::Failed],
                ..Default::default()
            })
        );
    }

    #[test]
    fn unknown_status_is_rejected_at_parse_time() {
        // The status vocabulary is pinned by the vendored schema, so an unknown
        // value is a definite user error caught at the flag boundary, and the
        // failure names the accepted values.
        let failure = list()
            .to_options()
            .run_inner(&["--status", "garbage"][..])
            .expect_err("expected an unknown --status to fail parsing");
        // bpaf line-wraps the rendered failure, so compare with newlines
        // collapsed to spaces.
        let message = failure.unwrap_stderr().replace('\n', " ");
        assert!(
            message.contains("Invalid status 'garbage'"),
            "unexpected parse failure: {message}"
        );
        assert!(
            message.contains(
                "valid values are: pending, running, completed, failed, timed_out, cancelled"
            ),
            "unexpected parse failure: {message}"
        );
    }
}
