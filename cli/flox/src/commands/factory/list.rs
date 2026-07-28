use std::fmt;
use std::str::FromStr;

use anyhow::{Result, anyhow, bail};
use bpaf::Bpaf;
use chrono::{DateTime, Months, TimeDelta, Utc};
use floxhub_client::{
    AttrPathItem,
    BuildFilters,
    BuildResponse,
    EffectiveBuildStatus,
    FactoryClientTrait,
    SourceCommitShaItem,
    SystemItem,
};
use interim::{Dialect, Interval, parse_date_string, parse_duration};
use serde::Serialize;
use tracing::instrument;

use super::{effective_status, effective_updated_at};
use crate::subcommand_metric;
use crate::utils::message::page_output;

/// List Flox Factory builds.
///
/// Each filter except --since is repeatable and ORs its values; different
/// filters AND together. An unfiltered invocation lists every build.
#[derive(Debug, Clone, PartialEq, Bpaf)]
pub struct List {
    /// Filter by build status; repeat to match any of several.
    /// Valid values: pending, running, completed, failed, timed_out,
    /// cancelled.
    #[bpaf(long, argument::<String>("STATUS"), parse(parse_status), many)]
    pub status: Vec<EffectiveBuildStatus>,

    /// Filter by system; repeat to match any of several.
    /// Examples: aarch64-darwin, aarch64-linux, x86_64-darwin, x86_64-linux.
    /// A system the server does not know is reported with the values it does.
    #[bpaf(long, argument::<String>("SYSTEM"), parse(parse_non_empty("System")), many)]
    pub system: Vec<SystemItem>,

    /// Filter by attribute path; a prefix matches; repeat to match any of
    /// several.
    #[bpaf(long, argument::<String>("PATH"), parse(parse_non_empty("Attribute path")), many)]
    pub attr_path: Vec<AttrPathItem>,

    /// Filter by source revision (commit SHA); a prefix matches; repeat to
    /// match any of several.
    #[bpaf(long("source-rev"), argument::<String>("REV"), parse(parse_non_empty("Source revision")), many)]
    pub source_commit_sha: Vec<SourceCommitShaItem>,

    /// Only builds created at or after this time, given as a duration counted
    /// back from now ("7d"), a phrase ("yesterday"), or an ISO 8601 timestamp.
    #[bpaf(long, argument::<String>("TIME"), parse(parse_since), optional)]
    pub since: Option<DateTime<Utc>>,

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
            system: self.system,
            attr_path: self.attr_path,
            source_commit_sha: self.source_commit_sha,
            since: self.since,
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

/// Parse one `--status` value into a typed [`EffectiveBuildStatus`] at the
/// CLI boundary. The type's `FromStr` is strict and its error names the
/// accepted values, so the message is shown as-is.
fn parse_status(s: String) -> Result<EffectiveBuildStatus> {
    Ok(s.parse()?)
}

/// Build a bpaf `parse` function for one flag value, naming `noun` in either
/// failure so a user passing several empty values can tell which flag was
/// rejected.
///
/// The empty case gets its own message; any other constraint the parsed type
/// enforces reports the type's own error, so the message stays truthful if
/// the schema tightens.
fn parse_non_empty<T>(noun: &'static str) -> impl Fn(String) -> Result<T>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    move |s: String| {
        if s.is_empty() {
            bail!("{noun} must not be empty.");
        }
        s.parse()
            .map_err(|e| anyhow!("{noun} '{s}' is invalid: {e}."))
    }
}

/// Fixes the reading of an ambiguous slash-date: `01/07/2026` is 7 January.
const SINCE_DIALECT: Dialect = Dialect::Us;

/// Parse the `--since` value at the CLI boundary, against the current time.
fn parse_since(s: String) -> Result<DateTime<Utc>> {
    resolve_since(&s, Utc::now())
}

/// Resolve a `--since` expression to the instant it names, relative to `now`.
///
/// A duration counts backwards, so `7d` and `7 days ago` name the same bound;
/// anything else is read as an absolute date or timestamp. Resolving here means
/// every page of one listing is sent the same bound, rather than a moving one.
/// The bound must not lie in the future, which no build could satisfy.
fn resolve_since(s: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    if s.is_empty() {
        bail!("Time must not be empty.");
    }

    let resolved = match parse_duration(s) {
        Ok(interval) => {
            back_from(now, interval).ok_or_else(|| anyhow!("Time '{s}' is out of range."))?
        },
        Err(_) => parse_date_string(s, now, SINCE_DIALECT).map_err(|_| {
            anyhow!("Time '{s}' is invalid; use a duration like '7d', a phrase like 'yesterday', or an ISO 8601 timestamp.")
        })?,
    };

    if resolved > now {
        bail!("Time '{s}' is in the future; '--since' names a point in the past.");
    }

    Ok(resolved)
}

/// Move `now` back by the interval's magnitude, whichever sign it carries.
/// Months are subtracted as calendar months, so the day of the month and the
/// time of day both survive.
fn back_from(now: DateTime<Utc>, interval: Interval) -> Option<DateTime<Utc>> {
    match interval {
        Interval::Seconds(seconds) => {
            now.checked_sub_signed(TimeDelta::try_seconds(seconds.unsigned_abs().into())?)
        },
        Interval::Days(days) => {
            now.checked_sub_signed(TimeDelta::try_days(days.unsigned_abs().into())?)
        },
        Interval::Months(months) => now.checked_sub_months(Months::new(months.unsigned_abs())),
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
            BUILD ID  ATTR PATH  SYSTEM          STATUS   UPDATED
            1         hello      x86_64-linux    running  2025-01-01T00:00:01+00:00
            2         ripgrep    aarch64-darwin  pending  2025-01-01T00:00:00+00:00
        "});
    }

    #[test]
    fn list_display_renders_new_status_labels() {
        // The labels introduced with the typed status: a timed-out build reads
        // `timed_out` (not `failed`), a pre-dispatch cancel reads `cancelled`
        // (not `pending`), and an unrecognized status renders tolerantly as
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
    async fn list_handler_forwards_all_filters() {
        let client = StubFactoryClient::with_outcomes(
            StubResult::Build(EffectiveBuildStatus::Completed),
            StubResult::NotFound,
        );
        let args = List {
            status: vec![EffectiveBuildStatus::Running, EffectiveBuildStatus::Failed],
            system: vec!["aarch64-darwin".parse().unwrap()],
            attr_path: vec!["hello".parse().unwrap()],
            source_commit_sha: vec!["abc123".parse().unwrap()],
            since: Some("2026-07-17T12:00:00Z".parse().unwrap()),
            json: false,
            no_pager: true,
        };

        args.handle(&client).await.unwrap();

        assert_eq!(
            client.last_filters(),
            Some(BuildFilters {
                status: vec![EffectiveBuildStatus::Running, EffectiveBuildStatus::Failed],
                system: vec!["aarch64-darwin".parse().unwrap()],
                attr_path: vec!["hello".parse().unwrap()],
                source_commit_sha: vec!["abc123".parse().unwrap()],
                since: Some("2026-07-17T12:00:00Z".parse().unwrap()),
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

    /// Every filter rejects an empty value at the boundary, before it becomes
    /// an unmatchable filter or a doomed request, and each names itself so a
    /// user passing several empty values can tell which one was rejected.
    #[test]
    fn empty_filter_values_are_rejected_and_name_their_flag() {
        // `--since` is optional rather than repeatable, so its boundary parse
        // applies inside the optional lift; it is covered here to pin that the
        // lift does not skip the parse.
        let cases = [
            ("--attr-path", "Attribute path must not be empty."),
            ("--source-rev", "Source revision must not be empty."),
            ("--system", "System must not be empty."),
            ("--since", "Time must not be empty."),
        ];

        for (flag, expected) in cases {
            let Err(failure) = list().to_options().run_inner(&[flag, ""][..]) else {
                panic!("expected an empty {flag} to fail parsing");
            };
            let message = failure.unwrap_stderr().replace('\n', " ");
            assert!(
                message.contains(expected),
                "expected {flag} to report {expected:?}, got: {message}"
            );
        }
    }

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().expect("test timestamp should parse")
    }

    /// The clock the resolver tests run against, so they check its arithmetic
    /// rather than the machine's current time. A Friday, strictly after
    /// midnight, which [`a_time_that_cannot_be_used_is_rejected_with_a_reason`]
    /// relies on: a same-weekday name resolves to today's midnight and only
    /// advances a week once that midnight is already behind `now`.
    fn now() -> DateTime<Utc> {
        utc("2026-07-24T12:00:00Z")
    }

    /// `--since` names a lower bound in the past, so a duration is an offset
    /// backwards from now however it is spelled.
    #[test]
    fn a_duration_counts_backwards_from_now() {
        let resolved: Vec<_> = ["30s", "15m", "2h", "7d", "3w", "7 days", "7 days ago"]
            .iter()
            .map(|input| resolve_since(input, now()).map_err(|e| e.to_string()))
            .collect();

        assert_eq!(resolved, vec![
            Ok(utc("2026-07-24T11:59:30Z")),
            Ok(utc("2026-07-24T11:45:00Z")),
            Ok(utc("2026-07-24T10:00:00Z")),
            Ok(utc("2026-07-17T12:00:00Z")),
            Ok(utc("2026-07-03T12:00:00Z")),
            Ok(utc("2026-07-17T12:00:00Z")),
            Ok(utc("2026-07-17T12:00:00Z")),
        ]);
    }

    /// A year and a month are calendar quantities rather than fixed spans of
    /// days, and neither disturbs the time of day.
    #[test]
    fn a_year_and_a_month_are_calendar_quantities() {
        let resolved: Vec<_> = ["1y", "1 month", "3 months ago"]
            .iter()
            .map(|input| resolve_since(input, now()).map_err(|e| e.to_string()))
            .collect();

        assert_eq!(resolved, vec![
            Ok(utc("2025-07-24T12:00:00Z")),
            Ok(utc("2026-06-24T12:00:00Z")),
            Ok(utc("2026-04-24T12:00:00Z")),
        ]);
    }

    /// Anything that is not a duration names an instant outright. A value
    /// without an offset is read as UTC.
    #[test]
    fn an_absolute_value_names_its_own_instant() {
        let resolved: Vec<_> = [
            "2026-07-01",
            "2026-07-01T06:30:00Z",
            "2026-07-01T06:30:00+02:00",
            "2026-07-01 06:30",
        ]
        .iter()
        .map(|input| resolve_since(input, now()).map_err(|e| e.to_string()))
        .collect();

        assert_eq!(resolved, vec![
            Ok(utc("2026-07-01T00:00:00Z")),
            Ok(utc("2026-07-01T06:30:00Z")),
            Ok(utc("2026-07-01T04:30:00Z")),
            Ok(utc("2026-07-01T06:30:00Z")),
        ]);
    }

    /// Each rejection says what is wrong with the value and offers a spelling
    /// that works, without exposing the parser's own diagnostics.
    #[test]
    fn a_time_that_cannot_be_used_is_rejected_with_a_reason() {
        let failures: Vec<_> = ["banana", "friday", "2027-01-01"]
            .iter()
            .map(|input| resolve_since(input, now()).unwrap_err().to_string())
            .collect();

        assert_eq!(failures, vec![
            "Time 'banana' is invalid; use a duration like '7d', a phrase like 'yesterday', or an ISO 8601 timestamp.".to_string(),
            "Time 'friday' is in the future; '--since' names a point in the past.".to_string(),
            "Time '2027-01-01' is in the future; '--since' names a point in the past.".to_string(),
        ]);
    }
}
