//! `flox factory` command namespace: read-only build inspection for operators,
//! plus the destructive `cancel` verb.
//!
//! Hidden from `flox --help`; discoverable via the operator runbook.

mod cancel;
mod list;
mod logs;
mod status;

use anyhow::{Result, anyhow};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use floxhub_client::{BuildResponse, FactoryClientError};
use indoc::indoc;
use tracing::instrument;

use crate::commands::display_help;

/// The label shown for an undispatched build's effective status.
///
/// A build with no task has never been dispatched to Build Coordinator, so it
/// has no task lifecycle status. `pending` is synthesized by the CLI — it is
/// NOT a value of the API `Status` enum (`queued`, `dispatching`, `running`,
/// `completed`, `failed`, `timed_out`, `cancelled`), so the "(not dispatched)"
/// suffix keeps a user from typing it as a `--status` filter and getting an
/// empty result or 422.
const UNDISPATCHED_STATUS_LABEL: &str = "pending (not dispatched)";

/// Compute the effective status string for a build.
///
/// Returns the task's lifecycle status when the build has been dispatched
/// (a task exists), otherwise the synthesized undispatched label. An
/// undispatched build always reads as pending — never as a terminal status —
/// because no task lifecycle has begun.
fn effective_status(build: &BuildResponse) -> String {
    build
        .task
        .as_ref()
        .map(|task| task.status.clone())
        .unwrap_or_else(|| UNDISPATCHED_STATUS_LABEL.to_string())
}

/// Compute the effective "last updated" timestamp for a build, as an RFC 3339
/// string.
///
/// A dispatched build reports its task's `updated_at`, which tracks lifecycle
/// progress. An undispatched build has no task, so it falls back to the
/// build's own `created_at`. This is the "updated, or created if never
/// updated" value shown in the `UPDATED` column.
fn effective_updated_at(build: &BuildResponse) -> String {
    build
        .task
        .as_ref()
        .map(|task| task.updated_at)
        .unwrap_or(build.created_at)
        .to_rfc3339()
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
/// Operator subcommands for inspecting Flox Factory builds.
#[derive(Debug, Clone, Bpaf)]
pub enum FactoryCommands {
    /// Print help information
    #[bpaf(command, hide)]
    Help,

    /// Show the status of a single Flox Factory build
    #[bpaf(command)]
    Status(#[bpaf(external(status::status))] status::Status),

    /// List Flox Factory builds
    #[bpaf(command)]
    List(#[bpaf(external(list::list))] list::List),

    /// Print the logs for a single Flox Factory build
    #[bpaf(command)]
    Logs(#[bpaf(external(logs::logs))] logs::Logs),

    /// Cancel a single Flox Factory build
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
                display_help(Some("factory".to_string()));
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

    use std::sync::atomic::{AtomicBool, Ordering};

    use chrono::TimeZone;
    use factory_api_v1::types::TaskResponse;
    use floxhub_client::{
        BuildResponse,
        FactoryApiError,
        FactoryByteStream,
        FactoryClientError,
        FactoryClientTrait,
        FactoryErrorResponse,
    };

    /// Construct a `BuildResponse` fixture. A `Some(task_status)` attaches a
    /// dispatch task with that lifecycle status; `None` leaves the build
    /// undispatched (no task).
    pub fn make_build(
        id: i64,
        system: &str,
        attr_path: &str,
        task_status: Option<&str>,
    ) -> BuildResponse {
        let task = task_status.map(|s| TaskResponse {
            task_id: id,
            task_type: "dispatch_build".to_string(),
            status: s.to_string(),
            created_at: chrono::Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 1).unwrap(),
            completed_at: None,
            error_class: None,
            error_message: None,
            started_at: None,
        });

        BuildResponse {
            build_id: id,
            // Mirrors the server's effective status: the task lifecycle status
            // when dispatched, else the pre-dispatch default of "pending".
            status: task_status.unwrap_or("pending").to_string(),
            system: system.to_string(),
            attr_path: attr_path.to_string(),
            catalog_name: "my-catalog".to_string(),
            build_type: "nixpkgs".to_string(),
            source_repo_url: "https://github.com/example/repo".to_string(),
            source_commit_sha: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".to_string(),
            nixpkgs_revision: "deadbeef1234567890deadbeef1234567890dead".to_string(),
            created_at: chrono::Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            exit_code: None,
            task,
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
        /// Return a build whose top-level `status` is this string.
        Build(String),
        /// Return [`FactoryClientError::NotFound`].
        NotFound,
        /// Return [`FactoryClientError::AuthRejected`].
        AuthRejected,
        /// Return [`FactoryClientError::Server`].
        Server,
    }

    impl StubResult {
        fn realize(&self) -> Result<BuildResponse, FactoryClientError> {
            match self {
                // Build the fixture coherently: `Some(status)` sets both the
                // task lifecycle status and the top-level status, preserving
                // `make_build`'s invariant rather than mutating after the fact.
                StubResult::Build(status) => {
                    Ok(make_build(1, "x86_64-linux", "hello", Some(status)))
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
    /// Drives `get_build` (the `status` lookup) and `cancel_build` (the DELETE)
    /// to independent [`StubResult`] outcomes, and records whether the DELETE was
    /// issued so a cancel test can assert the range guard short-circuits before
    /// it.
    pub struct StubFactoryClient {
        get_build: StubResult,
        cancel_build: StubResult,
        cancel_called: AtomicBool,
    }

    impl StubFactoryClient {
        /// A stub whose every lookup reports the build as missing.
        pub fn with_not_found() -> Self {
            Self::with_outcomes(StubResult::NotFound, StubResult::NotFound)
        }

        /// A stub whose `get_build` (the `status` lookup) yields `get_build` and
        /// whose DELETE (`cancel_build`) yields `cancel_build`.
        pub fn with_outcomes(get_build: StubResult, cancel_build: StubResult) -> Self {
            Self {
                get_build,
                cancel_build,
                cancel_called: AtomicBool::new(false),
            }
        }

        /// A stub exercising only the DELETE (`cancel_build`). The `cancel` verb
        /// issues no `get_build`, so that slot is an inert `NotFound` placeholder.
        pub fn with_cancel(cancel_build: StubResult) -> Self {
            Self::with_outcomes(StubResult::NotFound, cancel_build)
        }

        /// Whether `cancel_build` (the DELETE) was ever invoked.
        pub fn cancel_was_called(&self) -> bool {
            self.cancel_called.load(Ordering::SeqCst)
        }
    }

    impl FactoryClientTrait for StubFactoryClient {
        async fn list_builds(
            &self,
            _status: Option<&str>,
        ) -> Result<floxhub_client::ResultsPage<BuildResponse>, FactoryClientError> {
            // Mirror the get_build outcome's error, if any, so a not-found stub
            // stays not-found across endpoints; otherwise an empty page.
            self.get_build.realize()?;
            Ok(floxhub_client::ResultsPage {
                results: vec![],
                count: Some(0),
            })
        }

        async fn get_build(&self, _build_id: i64) -> Result<BuildResponse, FactoryClientError> {
            self.get_build.realize()
        }

        async fn cancel_build(&self, _build_id: i64) -> Result<BuildResponse, FactoryClientError> {
            self.cancel_called.store(true, Ordering::SeqCst);
            self.cancel_build.realize()
        }

        async fn get_build_logs(
            &self,
            _build_id: i64,
        ) -> Result<FactoryByteStream, FactoryClientError> {
            // Mirror the get_build outcome's error, if any, so a not-found stub
            // stays not-found across endpoints; otherwise serve the canned stream.
            self.get_build.realize()?;
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
mod error_tests {
    use floxhub_client::{FactoryApiError, FactoryClientError};
    use indoc::indoc;

    use super::user_facing_error;

    #[test]
    fn not_found_renders_the_verb_message() {
        let err = user_facing_error(
            FactoryClientError::NotFound,
            Some("No Flox Factory build found with ID 7.".to_string()),
        );
        assert_eq!(err.to_string(), "No Flox Factory build found with ID 7.");
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
