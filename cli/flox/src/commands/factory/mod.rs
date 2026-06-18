//! `flox factory` command namespace: read-only build inspection for operators.
//!
//! Hidden from `flox --help`; discoverable via the operator runbook. `Logs`
//! (ECO-99) and `Cancel` (ECO-100) are future verbs.

mod list;
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
    // ECO-99 will add: Logs(#[bpaf(external(logs::logs))] logs::Logs),
    // ECO-100 will add: Cancel(#[bpaf(external(cancel::cancel))] cancel::Cancel),
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
        }
    }
}

#[cfg(test)]
pub(crate) mod test_helpers {
    //! Shared test fixtures for the `status` and `list` verb handler tests.

    use chrono::TimeZone;
    use factory_api_v1::types::TaskResponse;
    use floxhub_client::{
        BuildResponse,
        FactoryByteStream,
        FactoryClientError,
        FactoryClientTrait,
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

    /// In-test stub implementing [`FactoryClientTrait`], used to drive the
    /// `status` handler down its not-found path without a live service.
    pub struct StubFactoryClient {
        not_found: bool,
    }

    impl StubFactoryClient {
        pub fn with_not_found() -> Self {
            Self { not_found: true }
        }
    }

    impl FactoryClientTrait for StubFactoryClient {
        async fn list_builds(
            &self,
            _status: Option<&str>,
        ) -> Result<floxhub_client::ResultsPage<BuildResponse>, FactoryClientError> {
            if self.not_found {
                return Err(FactoryClientError::NotFound);
            }
            Ok(floxhub_client::ResultsPage {
                results: vec![],
                count: Some(0),
            })
        }

        async fn get_build(&self, _build_id: i64) -> Result<BuildResponse, FactoryClientError> {
            if self.not_found {
                return Err(FactoryClientError::NotFound);
            }
            Ok(make_build(1, "x86_64-linux", "hello", Some("running")))
        }

        async fn get_build_logs(
            &self,
            _build_id: i64,
        ) -> Result<FactoryByteStream, FactoryClientError> {
            unimplemented!("StubFactoryClient does not implement get_build_logs")
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
