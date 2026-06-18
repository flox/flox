use std::fmt;
use std::str::FromStr;

use anyhow::Result;
use bpaf::Bpaf;
use floxhub_client::{BuildResponse, FactoryClientTrait, FactoryStatus};
use serde::Serialize;
use tracing::instrument;

use super::{effective_status, effective_updated_at};
use crate::subcommand_metric;
use crate::utils::message::page_output;

/// CLI-boundary filter for `--status`, extending the seven wire `Status`
/// values with the server-side keyword `undispatched` (builds where
/// `task_id IS NULL`).
///
/// `undispatched` is accepted by the GET /builds query parameter but is NOT
/// a value of the `Status` enum. It is the mechanism for filtering builds
/// the CLI renders as "pending (not dispatched)". The richer list-query
/// contract — including effective-status matching semantics and what
/// `undispatched` precisely means — is owned by ECO-97.
#[derive(Debug, Clone, PartialEq)]
pub enum StatusFilter {
    /// One of the seven task lifecycle statuses on the wire.
    Status(FactoryStatus),
    /// Server-side keyword: builds with no dispatched task (`task_id IS NULL`).
    Undispatched,
}

impl FromStr for StatusFilter {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "undispatched" {
            return Ok(StatusFilter::Undispatched);
        }
        FactoryStatus::from_str(s)
            .map(StatusFilter::Status)
            .map_err(|_| {
                format!("Invalid status '{s}'; valid values are: queued, dispatching, running, completed, failed, timed_out, cancelled, undispatched.")
            })
    }
}

impl fmt::Display for StatusFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StatusFilter::Status(s) => fmt::Display::fmt(s, f),
            StatusFilter::Undispatched => f.write_str("undispatched"),
        }
    }
}

/// List Flox Factory builds.
#[derive(Debug, Clone, PartialEq, Bpaf)]
pub struct List {
    /// Filter by build status.
    /// Valid values: queued, dispatching, running, completed, failed,
    /// timed_out, cancelled, undispatched.
    /// Use "undispatched" to show builds that have not yet been sent
    /// to Build Coordinator (displayed as "pending (not dispatched)").
    #[bpaf(long)]
    pub status: Option<StatusFilter>,

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

        // Convert the validated filter to its wire string. The seven Status
        // values use their own Display ("running", etc.); "undispatched" is
        // forwarded as the literal keyword the server accepts.
        let status_str = self.status.as_ref().map(|f| f.to_string());

        // Depage the full result set, mirroring `flox generations list`: the
        // operator sees every matching build at once and scrolls with the
        // pager, rather than stepping server-side pages by hand.
        let builds = client
            .list_builds(status_str.as_deref())
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
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::commands::factory::test_helpers::make_build;

    #[test]
    fn status_filter_parses_undispatched_keyword() {
        let f: StatusFilter = "undispatched".parse().unwrap();
        assert_eq!(f, StatusFilter::Undispatched);
    }

    #[test]
    fn status_filter_rejects_invalid_value_with_named_values() {
        let err = "garbage".parse::<StatusFilter>().unwrap_err();
        assert_eq!(
            err,
            "Invalid status 'garbage'; valid values are: queued, dispatching, running, completed, failed, timed_out, cancelled, undispatched."
        );
    }

    #[test]
    fn list_display_renders_table_exactly() {
        // A dispatched build shows its task's updated_at; an undispatched build
        // has no task, so UPDATED falls back to the build's created_at.
        let builds = vec![
            make_build(1, "x86_64-linux", "hello", Some("running")),
            make_build(2, "aarch64-darwin", "ripgrep", None),
        ];
        let display = BuildListDisplay::from(builds);
        assert_eq!(display.to_string(), indoc! {"
            BUILD ID  ATTR PATH  SYSTEM          STATUS                    UPDATED
            1         hello      x86_64-linux    running                   2025-01-01T00:00:01+00:00
            2         ripgrep    aarch64-darwin  pending (not dispatched)  2025-01-01T00:00:00+00:00
        "});
    }
}
