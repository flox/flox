//! `flox factory` command namespace: read-only inspection of Flox Factory runs
//! for operators, plus the destructive `cancel` verb.
//!
//! Hidden from `flox --help`; discoverable via the operator runbook.

mod cancel;
mod list;
mod logs;
mod status;

use anyhow::{Result, anyhow};
use bpaf::Bpaf;
use chrono::{DateTime, Utc};
use flox_rust_sdk::flox::Flox;
use floxhub_client::{
    EffectiveFactoryRunStatus,
    FactoryClientError,
    FactoryRunResponse,
    FactoryRunTrigger,
};
use indoc::indoc;
use tracing::instrument;

use crate::commands::display_command_help;

/// Compute the effective status label for a run from its server-computed
/// `status`.
///
/// An unrecognized future status renders as `unknown: <value>` rather than
/// blanking the row; every other status renders as its wire word.
fn effective_status(status: &EffectiveFactoryRunStatus) -> String {
    match status {
        EffectiveFactoryRunStatus::Unknown(value) => format!("unknown: {value}"),
        status => status.to_string(),
    }
}

/// Compute the effective "last updated" timestamp for a run.
///
/// A dispatched run reports its task's `updated_at`, which tracks lifecycle
/// progress. An undispatched run has no task, so it falls back to the
/// run's own `created_at`. This is the "updated, or created if never
/// updated" value shown in the `UPDATED` column.
fn effective_updated_at(run: &FactoryRunResponse) -> DateTime<Utc> {
    run.task
        .as_ref()
        .map(|task| task.updated_at)
        .unwrap_or(run.created_at)
}

/// Compute the label describing what scheduled a run.
fn trigger_label(trigger: Option<&FactoryRunTrigger>) -> String {
    let Some(trigger) = trigger else {
        return "-".to_string();
    };
    // The Factory sends a null kind for an event it does not classify.
    let kind = trigger.kind.as_deref().map_or("-", String::as_str);

    if trigger.stabilities.is_empty() {
        return kind.to_string();
    }

    // The stabilities that named the run's nixpkgs revision when it was
    // scheduled follow in parentheses. They explain why one package can be
    // built more than once per system. They sit inside the trigger label rather
    // than in a column of their own so they are not read as the revision's
    // current stability.
    format!("{kind} ({})", trigger.stabilities.join(","))
}

/// Rewrite a [`FactoryClientError`] as a product-level error, so an operator
/// never sees raw client or transport output.
///
/// The transport case is verb-independent. The not-found case is not: the
/// client does not know what the caller asked for, so `not_found` carries the
/// verb-specific message. A verb that cannot meaningfully 404 passes `None`.
fn user_facing_error(err: FactoryClientError, not_found: Option<String>) -> anyhow::Error {
    match err {
        FactoryClientError::Transport(_) => anyhow!(indoc! {"
            Could not reach the Flox Factory.
            Check your network connection and try again."}),
        FactoryClientError::NotFound => match not_found {
            Some(message) => anyhow!(message),
            None => anyhow!("The requested Flox Factory resource was not found."),
        },
        other => other.into(),
    }
}

// The verbs are thin wrappers over the `factory-api-v1` client. The namespace
// is hidden from top-level help via `#[bpaf(hide)]` on the hosting
// `Commands::Factory` variant; see the module docs above. These notes are kept
// as plain comments rather than a `///` doc comment because bpaf renders the
// enum's doc comment into `flox factory --help`, where implementation detail
// does not belong.
/// Operator subcommands for inspecting Flox Factory runs.
#[derive(Debug, Clone, Bpaf)]
pub enum FactoryCommands {
    /// Print help information
    #[bpaf(command, hide)]
    Help,

    /// Show the status of a single Flox Factory run
    #[bpaf(command)]
    Status(#[bpaf(external(status::status))] status::Status),

    /// List Flox Factory runs
    #[bpaf(command)]
    List(#[bpaf(external(list::list))] list::List),

    /// Print the logs for a single Flox Factory run
    #[bpaf(command)]
    Logs(#[bpaf(external(logs::logs))] logs::Logs),

    /// Cancel a single Flox Factory run
    #[bpaf(command)]
    Cancel(#[bpaf(external(cancel::cancel))] cancel::Cancel),
}

impl FactoryCommands {
    #[instrument(name = "factory", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        // The factory client shares its base URL with the catalog client, so
        // `FLOX_CATALOG_URL` already repoints both for local development; no
        // factory-specific override is needed.
        match self {
            FactoryCommands::Help => {
                display_command_help(Some("factory".to_string()));
                Ok(())
            },
            FactoryCommands::Status(args) => args.handle(&flox.floxhub_client).await,
            FactoryCommands::List(args) => args.handle(&flox.floxhub_client).await,
            FactoryCommands::Logs(args) => args.handle(&flox.floxhub_client).await,
            FactoryCommands::Cancel(args) => args.handle(&flox.floxhub_client).await,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_helpers {
    //! Shared test fixtures for the factory verb handler tests.

    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use chrono::TimeZone;
    use factory_api_v1::types::{TaskErrorClass, TaskResponse, TaskStatus};
    use floxhub_client::{
        EffectiveFactoryRunStatus,
        FactoryApiError,
        FactoryByteStream,
        FactoryClientError,
        FactoryClientTrait,
        FactoryErrorResponse,
        FactoryRunDetailResponse,
        FactoryRunFilters,
        FactoryRunId,
        FactoryRunResponse,
    };

    /// Construct a `FactoryRunResponse` fixture whose effective `status` is the given
    /// value, defaulting to pre-dispatch `pending` for `None`.
    ///
    /// The attached task mirrors the server's persisted shape: a pending or
    /// pre-dispatch-cancelled run has no task; a timed-out run is persisted as
    /// a failed task with the `timeout` error class; a run in any other
    /// dispatched state has a task with the matching lifecycle status.
    pub fn make_run(
        id: i64,
        system: &str,
        attr_path: &str,
        status: Option<EffectiveFactoryRunStatus>,
    ) -> FactoryRunResponse {
        let status = status.unwrap_or(EffectiveFactoryRunStatus::Pending);

        let task_footprint = match &status {
            EffectiveFactoryRunStatus::Running => Some((TaskStatus::Running, None)),
            EffectiveFactoryRunStatus::Completed => Some((TaskStatus::Completed, None)),
            EffectiveFactoryRunStatus::Failed => Some((TaskStatus::Failed, None)),
            EffectiveFactoryRunStatus::TimedOut => {
                Some((TaskStatus::Failed, Some(TaskErrorClass::Timeout)))
            },
            EffectiveFactoryRunStatus::Pending
            | EffectiveFactoryRunStatus::Cancelled
            | EffectiveFactoryRunStatus::Unknown(_) => None,
        };

        let task = task_footprint.map(|(task_status, error_class)| TaskResponse {
            task_id: id,
            task_type: "dispatch_build".to_string(),
            status: task_status,
            created_at: chrono::Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 1).unwrap(),
            completed_at: None,
            error_class,
            error_message: None,
            started_at: None,
        });

        FactoryRunResponse {
            run_id: id,
            status,
            system: system.to_string(),
            attr_path: attr_path.to_string(),
            catalog_name: "my-catalog".to_string(),
            build_type: "nixpkgs".to_string(),
            source_repo_url: "https://github.com/example/repo".to_string(),
            source_commit_sha: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".to_string(),
            nixpkgs_revision: "deadbeef1234567890deadbeef1234567890dead".to_string(),
            created_at: chrono::Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            exit_code: None,
            published_package: None,
            task,
            // A run from a Factory that predates trigger reporting; tests
            // covering the trigger set it on the result.
            trigger: None,
        }
    }

    /// Extend a `FactoryRunResponse` fixture into the `FactoryRunDetailResponse` that
    /// `get_factory_run` returns, with no published outputs.
    ///
    /// The destructuring is exhaustive, so a field added to either type fails
    /// to compile here rather than going unset.
    pub fn make_run_detail(run: FactoryRunResponse) -> FactoryRunDetailResponse {
        let FactoryRunResponse {
            attr_path,
            run_id,
            build_type,
            catalog_name,
            created_at,
            exit_code,
            nixpkgs_revision,
            published_package,
            source_commit_sha,
            source_repo_url,
            status,
            system,
            task,
            trigger,
        } = run;

        FactoryRunDetailResponse {
            attr_path,
            run_id,
            build_type,
            catalog_name,
            created_at,
            exit_code,
            nixpkgs_revision,
            outputs: vec![],
            published_package,
            source_commit_sha,
            source_repo_url,
            status,
            system,
            task,
            trigger,
        }
    }

    /// The outcome a [`StubFactoryClient`] method yields.
    ///
    /// [`FactoryClientError`] is not `Clone` (its `Transport` case wraps a
    /// `reqwest::Error`), so the error cases are named here and the variant is
    /// built on demand rather than stored. `Transport` is omitted because it
    /// cannot be constructed without a live failed request; the cancel verb's
    /// transport path is covered by the pure classifier tests instead.
    #[derive(Clone)]
    pub enum StubResult {
        /// Return a run whose effective `status` is this value.
        Run(EffectiveFactoryRunStatus),
        /// Return [`FactoryClientError::NotFound`].
        NotFound,
        /// Return [`FactoryClientError::AuthRejected`].
        AuthRejected,
        /// Return [`FactoryClientError::Server`].
        Server,
    }

    impl StubResult {
        fn realize(&self) -> Result<FactoryRunResponse, FactoryClientError> {
            match self {
                // Build the fixture coherently from the effective status,
                // preserving `make_run`'s server-shaped invariant rather than
                // mutating after the fact.
                StubResult::Run(status) => {
                    Ok(make_run(1, "x86_64-linux", "hello", Some(status.clone())))
                },
                StubResult::NotFound => Err(FactoryClientError::NotFound),
                StubResult::AuthRejected => Err(FactoryClientError::AuthRejected(stub_api_error())),
                StubResult::Server => Err(FactoryClientError::Server(stub_api_error())),
            }
        }
    }

    /// A placeholder API error for the stub's typed-error outcomes. The cancel
    /// handler renders its own curated message for `AuthRejected`/`Server`, so
    /// the handler tests never inspect this payload (the client-layer detail
    /// assertion uses a mock, not the stub); it exists only to satisfy the
    /// variant shape now that those variants wrap the underlying error.
    fn stub_api_error() -> FactoryApiError<FactoryErrorResponse> {
        FactoryApiError::<FactoryErrorResponse>::InvalidRequest("stubbed factory error".to_string())
    }

    /// In-test stub implementing [`FactoryClientTrait`].
    ///
    /// Drives `get_factory_run` (the `status` lookup) and `cancel_factory_run` (the DELETE)
    /// to independent [`StubResult`] outcomes, records whether the DELETE was
    /// issued, and captures the last [`FactoryRunFilters`] passed to `list_factory_runs`
    /// so a list test can assert the CLI's filter translation without a mock
    /// server.
    pub struct StubFactoryClient {
        get_factory_run: StubResult,
        cancel_factory_run: StubResult,
        cancel_called: AtomicBool,
        last_filters: Mutex<Option<FactoryRunFilters>>,
    }

    impl StubFactoryClient {
        /// A stub whose every lookup reports the run as missing.
        pub fn with_not_found() -> Self {
            Self::with_outcomes(StubResult::NotFound, StubResult::NotFound)
        }

        /// A stub whose `get_factory_run` (the `status` lookup) yields `get_factory_run` and
        /// whose DELETE (`cancel_factory_run`) yields `cancel_factory_run`.
        pub fn with_outcomes(get_factory_run: StubResult, cancel_factory_run: StubResult) -> Self {
            Self {
                get_factory_run,
                cancel_factory_run,
                cancel_called: AtomicBool::new(false),
                last_filters: Mutex::new(None),
            }
        }

        /// A stub exercising only the DELETE (`cancel_factory_run`). The `cancel` verb
        /// issues no `get_factory_run`, so that slot is an inert `NotFound` placeholder.
        pub fn with_cancel(cancel_factory_run: StubResult) -> Self {
            Self::with_outcomes(StubResult::NotFound, cancel_factory_run)
        }

        /// Whether `cancel_factory_run` (the DELETE) was ever invoked.
        pub fn cancel_was_called(&self) -> bool {
            self.cancel_called.load(Ordering::SeqCst)
        }

        /// The [`FactoryRunFilters`] from the most recent `list_factory_runs` call, if any.
        pub fn last_filters(&self) -> Option<FactoryRunFilters> {
            self.last_filters.lock().unwrap().clone()
        }
    }

    impl FactoryClientTrait for StubFactoryClient {
        async fn list_factory_runs(
            &self,
            filters: &FactoryRunFilters,
        ) -> Result<floxhub_client::ResultsPage<FactoryRunResponse>, FactoryClientError> {
            *self.last_filters.lock().unwrap() = Some(filters.clone());
            // Mirror the get_factory_run outcome's error, if any, so a not-found stub
            // stays not-found across endpoints; otherwise an empty page.
            self.get_factory_run.realize()?;
            Ok(floxhub_client::ResultsPage {
                results: vec![],
                count: Some(0),
            })
        }

        async fn get_factory_run(
            &self,
            _run_id: FactoryRunId,
        ) -> Result<FactoryRunDetailResponse, FactoryClientError> {
            self.get_factory_run.realize().map(make_run_detail)
        }

        async fn cancel_factory_run(
            &self,
            _run_id: FactoryRunId,
        ) -> Result<FactoryRunResponse, FactoryClientError> {
            self.cancel_called.store(true, Ordering::SeqCst);
            self.cancel_factory_run.realize()
        }

        async fn get_factory_run_logs(
            &self,
            _run_id: FactoryRunId,
        ) -> Result<FactoryByteStream, FactoryClientError> {
            // Mirror the get_factory_run outcome's error, if any, so a not-found stub
            // stays not-found across endpoints; otherwise serve the canned stream.
            self.get_factory_run.realize()?;
            let lines: [&[u8]; 2] = [b"Building hello...\n", b"Build completed.\n"];
            let chunks = lines
                .into_iter()
                .map(|line| Ok::<_, reqwest::Error>(line.into()));
            Ok(FactoryByteStream::new(Box::pin(futures::stream::iter(
                chunks,
            ))))
        }
    }
}

#[cfg(test)]
mod parser_tests {
    //! Guard the one `bpaf` footgun in this namespace: a positional declared
    //! before a flag parses fine but panics when help is rendered, taking down
    //! every verb at once. Rendering the namespace help builds each verb's meta,
    //! so this catches the misordering for all current and future verbs.

    use bpaf::ParseFailure;

    use crate::commands::flox_cli;

    #[test]
    fn factory_help_renders_instead_of_panicking() {
        match flox_cli().run_inner(&["factory", "--help"]) {
            Err(ParseFailure::Stdout(..)) => {},
            Err(other) => panic!("expected factory help output, got error {other:?}"),
            Ok(_) => panic!("expected factory help output, parsed a command instead"),
        }
    }
}

#[cfg(test)]
mod trigger_label_tests {
    use floxhub_client::FactoryRunTrigger;
    use pretty_assertions::assert_eq;

    use super::trigger_label;

    #[test]
    fn trigger_label_combines_kind_and_stabilities() {
        let trigger = |kind: Option<&str>, stabilities: &[&str]| FactoryRunTrigger {
            kind: kind.map(|kind| kind.to_string().into()),
            stabilities: stabilities.iter().map(|s| s.to_string()).collect(),
        };
        let cases = [
            ("not reported", None, "-"),
            ("empty", Some(trigger(None, &[])), "-"),
            (
                "kind only",
                Some(trigger(Some("source_push"), &[])),
                "source_push",
            ),
            (
                "stabilities only",
                Some(trigger(None, &["staging"])),
                "- (staging)",
            ),
            (
                "kind and stabilities",
                Some(trigger(Some("source_push"), &["stable", "staging"])),
                "source_push (stable,staging)",
            ),
        ];

        let actual: Vec<_> = cases
            .iter()
            .map(|(name, trigger, _)| (*name, trigger_label(trigger.as_ref())))
            .collect();
        let expected: Vec<_> = cases
            .iter()
            .map(|(name, _, label)| (*name, label.to_string()))
            .collect();

        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod error_tests {
    use floxhub_client::{FactoryApiError, FactoryClientError};
    use indoc::indoc;

    use super::user_facing_error;

    #[test]
    fn not_found_renders_the_verb_message() {
        let err = user_facing_error(
            FactoryClientError::NotFound,
            Some("No Flox Factory run found with ID 7.".to_string()),
        );
        assert_eq!(err.to_string(), "No Flox Factory run found with ID 7.");
    }

    #[tokio::test]
    async fn transport_renders_the_unreachable_message() {
        // A deterministic transport failure: port 0 is never a valid
        // connection target, so the request always fails at the transport
        // layer, with no reliance on a particular port being closed.
        let transport_err = reqwest::get("http://127.0.0.1:0").await.unwrap_err();
        let err = user_facing_error(FactoryClientError::Transport(transport_err), None);
        assert_eq!(err.to_string(), indoc! {"
            Could not reach the Flox Factory.
            Check your network connection and try again."});
    }

    #[test]
    fn other_errors_keep_the_client_rendering() {
        let err = user_facing_error(
            FactoryClientError::APIError(FactoryApiError::InvalidRequest("boom".to_string())),
            None,
        );
        assert_eq!(err.to_string(), "Invalid Request: boom");
    }
}
