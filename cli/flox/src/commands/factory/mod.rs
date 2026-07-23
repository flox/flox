//! `flox factory` command namespace: read-only build inspection for operators,
//! plus the destructive `cancel` verb.
//!
//! Hidden from `flox --help`; discoverable via the operator runbook.

mod cancel;
mod list;
mod logs;
mod status;

use std::fmt;
use std::str::FromStr;

use anyhow::{Result, anyhow};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use floxhub_client::{BuildResponse, EffectiveBuildStatus, FactoryClientError};
use indoc::indoc;
use tracing::instrument;

use crate::commands::display_help;

/// A Flox Factory build ID.
///
/// Server build IDs are positive `i64`. Parsing at the CLI boundary rejects
/// zero, negative, non-numeric, and out-of-range values, so a verb only ever
/// holds an ID the server could have assigned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildId(i64);

impl BuildId {
    /// The inner build ID, a positive `i64`.
    pub fn get(self) -> i64 {
        self.0
    }
}

impl FromStr for BuildId {
    type Err = String;

    /// Accept only a positive `i64`. Parsing as `i64` first rejects values above
    /// `i64::MAX` as overflow rather than wrapping them onto the wire.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.parse::<i64>() {
            Ok(id) if id >= 1 => Ok(BuildId(id)),
            _ => Err(format!(
                "Invalid build ID '{s}'; expected a positive integer."
            )),
        }
    }
}

impl fmt::Display for BuildId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The label shown for a build whose effective status is `pending`.
///
/// A pending build has never been dispatched to Build Coordinator, so it has no
/// task lifecycle yet. The "(not dispatched)" suffix distinguishes it from an
/// in-flight build and signals that `pending` is the pre-dispatch state.
const UNDISPATCHED_STATUS_LABEL: &str = "pending (not dispatched)";

/// Compute the effective status label for a build from its server-computed
/// `status`.
///
/// `pending` renders with the "(not dispatched)" suffix; an unrecognized future
/// status renders as `unknown: <value>` rather than blanking the row; every
/// other status renders as its wire word.
fn effective_status(build: &BuildResponse) -> String {
    match &build.status {
        EffectiveBuildStatus::Pending => UNDISPATCHED_STATUS_LABEL.to_string(),
        EffectiveBuildStatus::Unknown(value) => format!("unknown: {value}"),
        status => status.to_string(),
    }
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

    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use chrono::TimeZone;
    use factory_api_v1::types::{TaskErrorClass, TaskResponse, TaskStatus};
    use floxhub_client::{
        BuildFilters,
        BuildResponse,
        EffectiveBuildStatus,
        FactoryApiError,
        FactoryByteStream,
        FactoryClientError,
        FactoryClientTrait,
        FactoryErrorResponse,
    };

    /// Construct a `BuildResponse` fixture whose effective `status` is the given
    /// value, defaulting to pre-dispatch `pending` for `None`.
    ///
    /// The attached task mirrors the server's persisted shape: a pending or
    /// pre-dispatch-cancelled build carries no task; a timed-out build is
    /// persisted as a failed task carrying the `timeout` error class; the other
    /// dispatched states carry the matching task lifecycle status.
    pub fn make_build(
        id: i64,
        system: &str,
        attr_path: &str,
        status: Option<EffectiveBuildStatus>,
    ) -> BuildResponse {
        let status = status.unwrap_or(EffectiveBuildStatus::Pending);

        let task_footprint = match &status {
            EffectiveBuildStatus::Running => Some((TaskStatus::Running, None)),
            EffectiveBuildStatus::Completed => Some((TaskStatus::Completed, None)),
            EffectiveBuildStatus::Failed => Some((TaskStatus::Failed, None)),
            EffectiveBuildStatus::TimedOut => {
                Some((TaskStatus::Failed, Some(TaskErrorClass::Timeout)))
            },
            EffectiveBuildStatus::Pending
            | EffectiveBuildStatus::Cancelled
            | EffectiveBuildStatus::Unknown(_) => None,
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

        BuildResponse {
            build_id: id,
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
        /// Return a build whose effective `status` is this value.
        Build(EffectiveBuildStatus),
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
                // Build the fixture coherently from the effective status,
                // preserving `make_build`'s server-shaped invariant rather than
                // mutating after the fact.
                StubResult::Build(status) => {
                    Ok(make_build(1, "x86_64-linux", "hello", Some(status.clone())))
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
    /// to independent [`StubResult`] outcomes, records whether the DELETE was
    /// issued, and captures the last [`BuildFilters`] passed to `list_builds`
    /// so a list test can assert the CLI's filter translation without a mock
    /// server.
    pub struct StubFactoryClient {
        get_build: StubResult,
        cancel_build: StubResult,
        cancel_called: AtomicBool,
        last_filters: Mutex<Option<BuildFilters>>,
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
                last_filters: Mutex::new(None),
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

        /// The [`BuildFilters`] from the most recent `list_builds` call, if any.
        pub fn last_filters(&self) -> Option<BuildFilters> {
            self.last_filters.lock().unwrap().clone()
        }
    }

    impl FactoryClientTrait for StubFactoryClient {
        async fn list_builds(
            &self,
            filters: &BuildFilters,
        ) -> Result<floxhub_client::ResultsPage<BuildResponse>, FactoryClientError> {
            *self.last_filters.lock().unwrap() = Some(filters.clone());
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
mod build_id_tests {
    use super::BuildId;

    #[test]
    fn accepts_the_positive_i64_range() {
        assert_eq!("1".parse::<BuildId>().map(BuildId::get), Ok(1));
        assert_eq!(
            i64::MAX.to_string().parse::<BuildId>().map(BuildId::get),
            Ok(i64::MAX)
        );
    }

    #[test]
    fn rejects_zero_negative_overflow_and_garbage() {
        assert!("0".parse::<BuildId>().is_err());
        assert!("-1".parse::<BuildId>().is_err());
        assert!(u64::MAX.to_string().parse::<BuildId>().is_err());
        assert!("abc".parse::<BuildId>().is_err());
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
