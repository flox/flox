use std::num::NonZeroU64;

use anyhow::Result;
use bpaf::Bpaf;
use floxhub_client::{BuildResponse, FactoryClientError, FactoryClientTrait, FactoryStatus};
use indoc::{formatdoc, indoc};
use tracing::instrument;

use crate::utils::message;
use crate::{Exit, subcommand_metric};

/// Cancel a single Flox Factory build.
///
/// Exit codes: 0 success, 1 service error, 2 not found, 4 unreachable,
/// 5 authentication rejected.
#[derive(Debug, Clone, PartialEq, Bpaf)]
pub struct Cancel {
    /// Display output as JSON
    #[bpaf(long)]
    pub json: bool,

    /// Build ID to cancel
    #[bpaf(positional("ID"))]
    pub id: NonZeroU64,
}

impl Cancel {
    #[instrument(name = "cancel", skip_all)]
    pub async fn handle(self, client: &impl FactoryClientTrait) -> Result<()> {
        subcommand_metric!("factory::cancel");

        // Build IDs are `i64` server-side, so an ID beyond `i64::MAX` cannot
        // name a real build. Reject it as not found rather than cast with `as`,
        // whose silent wrap would put a negative ID on the destructive endpoint;
        // this returns before any request is issued.
        let Ok(build_id) = i64::try_from(self.id.get()) else {
            return Err(self.fail(&FactoryClientError::NotFound));
        };

        // Issue the idempotent cancel. A 200 carries the build's effective
        // status, from which the outcome (initiated vs already terminal) is
        // read; every HTTP and transport failure is classified in the client
        // layer into the typed error the exit-code contract maps.
        match client.cancel_build(build_id).await {
            Ok(build) => match success_outcome(&build, self.id) {
                // Terminal: the cancel is fully resolved. Print to stdout, exit 0.
                CancelOutcome::Resolved(message) => {
                    print!("{}", render(&build, &message, self.json)?);
                    Ok(())
                },
                // Accepted but not yet terminal: the idempotent DELETE succeeded,
                // so exit 0 (automation must not retry). Warn about the caveat,
                // and still emit the body under `--json`.
                CancelOutcome::Accepted(message) => {
                    if self.json {
                        print!("{}", render(&build, &message, true)?);
                    }
                    message::warning(message);
                    Ok(())
                },
                // A 200 with a status outside the lifecycle vocabulary is a
                // service contract violation, not a successful cancel.
                CancelOutcome::Unexpected(message) => {
                    message::error(message);
                    Err(Exit(1).into())
                },
            },
            Err(err) => Err(self.fail(&err)),
        }
    }

    /// Print the error message for a failed step and return the carrying
    /// [`Exit`], so the process exit code alone tells automation what to do.
    fn fail(&self, err: &FactoryClientError) -> anyhow::Error {
        let (message, code) = classify_error(err, self.id);
        message::error(message);
        Exit(code).into()
    }
}

/// Map a client error to a user-facing message and exit code.
///
/// Pure, so the exit-code contract is tested exhaustively without driving I/O.
/// An unrecognised response degrades to a generic service error (exit 1),
/// following the codebase's graceful handling of unexpected responses rather
/// than escalating it to a bespoke outcome.
fn classify_error(err: &FactoryClientError, id: NonZeroU64) -> (String, u8) {
    match err {
        // No such build: exit 2.
        FactoryClientError::NotFound => (
            formatdoc! {"
                No Flox Factory build found with ID {id}.
                Use 'flox factory list' to see existing builds."},
            2,
        ),
        // Auth is never retryable: exit 5.
        FactoryClientError::AuthRejected(_) => (
            indoc! {"
                Authentication was rejected by the Flox Factory.
                Run 'flox auth login' and try again."}
            .to_string(),
            5,
        ),
        // No HTTP response at all: the service is unreachable. Exit 4.
        FactoryClientError::Transport(_) => (
            indoc! {"
                Could not reach the Flox Factory.
                Check your network connection and try again."}
            .to_string(),
            4,
        ),
        // A server-side error (5xx/422): the host answered over HTTP and erred,
        // so it is retryable. Exit 1; the cancel endpoint documents 502 as
        // retry-with-backoff.
        FactoryClientError::Server(_) => (
            formatdoc! {"
                The Flox Factory reported a server error for build {id}.
                This is usually temporary; wait a moment and try again."},
            1,
        ),
        // A non-auth 4xx, or a body that did not parse as a build: an
        // unrecognised response, reported as a generic service error and retried
        // rather than escalated. Exit 1. Naming the variant rather than `_` makes
        // a future error variant a compile error rather than a silent default.
        FactoryClientError::APIError(_) => (
            formatdoc! {"
                The Flox Factory could not cancel build {id}.
                This is usually temporary; wait a moment and try again."},
            1,
        ),
    }
}

/// The outcome of a successful (HTTP 200) cancel response, read from the
/// build's effective `status`.
#[derive(Debug, PartialEq)]
enum CancelOutcome {
    /// The build reached a terminal state; the cancel is fully resolved.
    /// Printed to stdout, exit 0.
    Resolved(String),
    /// The cancel was accepted but the build has not yet reached a terminal
    /// state. Surfaced as a warning, exit 0: the idempotent DELETE succeeded,
    /// so automation must not retry it.
    Accepted(String),
    /// A 200 whose status is outside the lifecycle vocabulary: a contract
    /// violation, not a successful cancel. Surfaced as an error, exit 1.
    Unexpected(String),
}

/// Classify a successful (HTTP 200) cancel response from the build's effective
/// `status`.
///
/// The seven wire statuses parse into [`FactoryStatus`]; matching the enum makes
/// a newly added server status a compile error here (once the client is
/// regenerated) rather than a silent contract violation. `pending` is the one
/// effective status the enum cannot represent: it is synthesized server-side for
/// an undispatched build (`task_id IS NULL`), so it is matched on the string.
///
/// A satisfied cancel reports a terminal `cancelled`/`completed`/`failed`/
/// `timed_out`. A non-terminal status (`running`/`queued`/`dispatching`/
/// `pending`) means the cancel was accepted but the build has not yet stopped;
/// like the read verbs, that is a normal outcome, not a retryable error. Only a
/// status outside the lifecycle vocabulary is a contract violation.
fn success_outcome(build: &BuildResponse, id: NonZeroU64) -> CancelOutcome {
    let status = build.status.as_str();
    match build.status.parse::<FactoryStatus>() {
        Ok(FactoryStatus::Cancelled) => {
            CancelOutcome::Resolved(format!("Cancellation initiated for build {id}."))
        },
        // `timed_out` is a terminal status in the factory `Status` vocabulary;
        // the service normalizes it to `failed` on a response (see the
        // `BuildResponse.status` docs), so it should not appear here, but a
        // known terminal status must not read as a contract violation.
        Ok(FactoryStatus::Completed | FactoryStatus::Failed | FactoryStatus::TimedOut) => {
            CancelOutcome::Resolved(format!(
                "Build {id} is already {status}; nothing to cancel."
            ))
        },
        Ok(FactoryStatus::Running | FactoryStatus::Queued | FactoryStatus::Dispatching) => {
            CancelOutcome::Accepted(accepted_message(id, status))
        },
        // `pending`: an undispatched build's synthesized status, outside the
        // `FactoryStatus` enum but a known, non-terminal outcome.
        Err(_) if status == "pending" => CancelOutcome::Accepted(accepted_message(id, status)),
        // A status outside the lifecycle vocabulary is a contract violation, not
        // a successful cancel.
        Err(_) => CancelOutcome::Unexpected(formatdoc! {"
            The Flox Factory returned an unexpected status '{status}' for build {id}.
            Run 'flox factory status {id}' to check the build."}),
    }
}

/// The "accepted but not yet terminal" message, shared by the non-terminal wire
/// statuses and the synthesized `pending`.
fn accepted_message(id: NonZeroU64, status: &str) -> String {
    formatdoc! {"
        Cancellation accepted for build {id}; it is {status} and not yet terminal.
        Run 'flox factory status {id}' to follow it."}
}

/// Render a successful cancel: the human outcome line, or the raw build as JSON
/// when `--json` is set (mirrors `status.rs`'s split).
fn render(build: &BuildResponse, outcome: &str, json: bool) -> Result<String> {
    if json {
        Ok(format!("{}\n", serde_json::to_string_pretty(build)?))
    } else {
        Ok(format!("{outcome}\n"))
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use floxhub_client::FactoryApiError;
    use pretty_assertions::assert_eq;
    use tracing::instrument::WithSubscriber;

    use super::*;
    use crate::commands::factory::test_helpers::{StubFactoryClient, StubResult, make_build};

    fn id(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).unwrap()
    }

    // -------------------------------------------------------------------------
    // success_outcome — the body-status -> outcome mapping
    // -------------------------------------------------------------------------

    fn build_with_status(status: &str) -> BuildResponse {
        make_build(42, "x86_64-linux", "hello", Some(status))
    }

    #[test]
    fn cancelled_status_is_resolved() {
        assert_eq!(
            success_outcome(&build_with_status("cancelled"), id(42)),
            CancelOutcome::Resolved("Cancellation initiated for build 42.".to_string())
        );
    }

    #[test]
    fn completed_status_is_resolved() {
        assert_eq!(
            success_outcome(&build_with_status("completed"), id(42)),
            CancelOutcome::Resolved(
                "Build 42 is already completed; nothing to cancel.".to_string()
            )
        );
    }

    #[test]
    fn failed_status_is_resolved() {
        assert_eq!(
            success_outcome(&build_with_status("failed"), id(42)),
            CancelOutcome::Resolved("Build 42 is already failed; nothing to cancel.".to_string())
        );
    }

    #[test]
    fn timed_out_status_is_resolved() {
        // `timed_out` is terminal, so it is an "already terminal" exit-0
        // outcome, not a contract violation.
        assert_eq!(
            success_outcome(&build_with_status("timed_out"), id(42)),
            CancelOutcome::Resolved(
                "Build 42 is already timed_out; nothing to cancel.".to_string()
            )
        );
    }

    #[test]
    fn non_terminal_status_is_accepted() {
        // A 200 with a non-terminal status means the cancel was accepted but the
        // build has not yet stopped: a normal exit-0 outcome, not a retry.
        assert_eq!(
            success_outcome(&build_with_status("running"), id(42)),
            CancelOutcome::Accepted(
                indoc! {"
                    Cancellation accepted for build 42; it is running and not yet terminal.
                    Run 'flox factory status 42' to follow it."}
                .to_string()
            )
        );
    }

    #[test]
    fn pending_status_is_accepted() {
        // `pending` is an undispatched build's synthesized status: not in the
        // `FactoryStatus` enum, but a known non-terminal outcome, so the cancel
        // is accepted (exit 0) rather than read as a contract violation.
        assert_eq!(
            success_outcome(&build_with_status("pending"), id(42)),
            CancelOutcome::Accepted(
                indoc! {"
                    Cancellation accepted for build 42; it is pending and not yet terminal.
                    Run 'flox factory status 42' to follow it."}
                .to_string()
            )
        );
    }

    #[test]
    fn out_of_vocabulary_status_is_unexpected() {
        // Only a status outside the lifecycle vocabulary is a contract violation.
        assert_eq!(
            success_outcome(&build_with_status("frobnicated"), id(42)),
            CancelOutcome::Unexpected(
                indoc! {"
                    The Flox Factory returned an unexpected status 'frobnicated' for build 42.
                    Run 'flox factory status 42' to check the build."}
                .to_string()
            )
        );
    }

    // -------------------------------------------------------------------------
    // classify_error — the exhaustive exit-code contract
    // -------------------------------------------------------------------------

    #[test]
    fn not_found_is_exit_2() {
        assert_eq!(
            classify_error(&FactoryClientError::NotFound, id(7)),
            (
                indoc! {"
                    No Flox Factory build found with ID 7.
                    Use 'flox factory list' to see existing builds."}
                .to_string(),
                2,
            )
        );
    }

    #[test]
    fn auth_rejected_is_exit_5() {
        let err =
            FactoryClientError::AuthRejected(FactoryApiError::InvalidRequest("auth".to_string()));
        assert_eq!(
            classify_error(&err, id(7)),
            (
                indoc! {"
                    Authentication was rejected by the Flox Factory.
                    Run 'flox auth login' and try again."}
                .to_string(),
                5,
            )
        );
    }

    #[tokio::test]
    async fn transport_is_exit_4() {
        // Port 0 is never a valid connection target, so the request always fails
        // at the transport layer regardless of which ports are closed.
        let transport = reqwest::get("http://127.0.0.1:0").await.unwrap_err();
        assert_eq!(
            classify_error(&FactoryClientError::Transport(transport), id(7)),
            (
                indoc! {"
                    Could not reach the Flox Factory.
                    Check your network connection and try again."}
                .to_string(),
                4,
            )
        );
    }

    #[test]
    fn server_is_service_error_exit_1() {
        // A 5xx is a retryable service fault; the cancel endpoint documents 502
        // as retry-with-backoff.
        let err = FactoryClientError::Server(FactoryApiError::InvalidRequest("5xx".to_string()));
        assert_eq!(
            classify_error(&err, id(7)),
            (
                indoc! {"
                    The Flox Factory reported a server error for build 7.
                    This is usually temporary; wait a moment and try again."}
                .to_string(),
                1,
            )
        );
    }

    #[test]
    fn unrecognised_response_degrades_to_service_error_exit_1() {
        // A non-auth 4xx, or a 200 whose body did not parse as a build, lands in
        // the generic API error and degrades to a retryable service error rather
        // than escalating to a bespoke outcome.
        let err = FactoryClientError::APIError(FactoryApiError::InvalidRequest("boom".to_string()));
        assert_eq!(
            classify_error(&err, id(7)),
            (
                indoc! {"
                    The Flox Factory could not cancel build 7.
                    This is usually temporary; wait a moment and try again."}
                .to_string(),
                1,
            )
        );
    }

    // -------------------------------------------------------------------------
    // render — the --json / human split
    // -------------------------------------------------------------------------

    #[test]
    fn render_human_prints_the_outcome_line() {
        let build = make_build(42, "x86_64-linux", "hello", Some("running"));
        assert_eq!(
            render(&build, "Cancellation initiated for build 42.", false).unwrap(),
            "Cancellation initiated for build 42.\n"
        );
    }

    #[test]
    fn render_json_emits_the_build_and_ignores_the_outcome() {
        let build = make_build(42, "x86_64-linux", "hello", Some("running"));
        let rendered = render(&build, "OUTCOME LINE", true).unwrap();
        // The JSON branch serializes the build and drops the human outcome
        // line. Assert the behavior of the branch, not serde's round-trip
        // fidelity (AGENTS.md).
        assert!(!rendered.contains("OUTCOME LINE"));
        assert!(rendered.contains("\"build_id\": 42"));
        assert!(rendered.contains("\"status\": \"running\""));
    }

    // -------------------------------------------------------------------------
    // handle — wiring the DELETE outcome to the exit-code contract
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn successful_cancel_issues_delete_and_returns_ok() {
        let client = StubFactoryClient::with_cancel(StubResult::Build("cancelled".to_string()));
        let args = Cancel {
            json: false,
            id: id(42),
        };

        let result = async { args.handle(&client).await }
            .with_subscriber(test_subscriber_message_only().0)
            .await;

        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert!(
            client.cancel_was_called(),
            "the DELETE should have been issued"
        );
    }

    #[tokio::test]
    async fn delete_failure_returns_exit_after_issuing_delete() {
        let client = StubFactoryClient::with_cancel(StubResult::Server);
        let args = Cancel {
            json: false,
            id: id(42),
        };

        let (subscriber, writer) = test_subscriber_message_only();
        let err = async { args.handle(&client).await.unwrap_err() }
            .with_subscriber(subscriber)
            .await;

        assert!(err.is::<Exit>(), "expected an Exit error, got {err:?}");
        assert!(
            client.cancel_was_called(),
            "the DELETE should have been issued"
        );
        assert_eq!(writer.to_string(), indoc! {"
            ✘ ERROR: The Flox Factory reported a server error for build 42.
            This is usually temporary; wait a moment and try again.
        "});
    }

    #[tokio::test]
    async fn delete_non_terminal_status_warns_and_exits_zero() {
        // A 200 with a non-terminal status means the cancel was accepted but the
        // build has not yet stopped: a normal success (exit 0) with a warning,
        // not a retryable error.
        let client = StubFactoryClient::with_cancel(StubResult::Build("running".to_string()));
        let args = Cancel {
            json: false,
            id: id(42),
        };

        let (subscriber, writer) = test_subscriber_message_only();
        let result = async { args.handle(&client).await }
            .with_subscriber(subscriber)
            .await;

        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert!(
            client.cancel_was_called(),
            "the DELETE should have been issued"
        );
        assert_eq!(writer.to_string(), indoc! {"
            ! Cancellation accepted for build 42; it is running and not yet terminal.
            Run 'flox factory status 42' to follow it.
        "});
    }

    #[tokio::test]
    async fn delete_out_of_vocabulary_status_is_a_service_error() {
        // A 200 whose status is outside the lifecycle vocabulary is a contract
        // violation, not a successful cancel: the verb returns exit 1 and emits
        // the error to stderr rather than printing the build to stdout.
        let client = StubFactoryClient::with_cancel(StubResult::Build("frobnicated".to_string()));
        let args = Cancel {
            json: false,
            id: id(42),
        };

        let (subscriber, writer) = test_subscriber_message_only();
        let err = async { args.handle(&client).await.unwrap_err() }
            .with_subscriber(subscriber)
            .await;

        assert!(err.is::<Exit>(), "expected an Exit error, got {err:?}");
        assert!(
            client.cancel_was_called(),
            "the DELETE should have been issued"
        );
        assert_eq!(writer.to_string(), indoc! {"
            ✘ ERROR: The Flox Factory returned an unexpected status 'frobnicated' for build 42.
            Run 'flox factory status 42' to check the build.
        "});
    }

    #[tokio::test]
    async fn delete_not_found_returns_exit_2() {
        // A DELETE against a missing build 404s; the client maps it to NotFound
        // and the verb exits 2. The DELETE is what produced the 404, so it was
        // issued.
        let client = StubFactoryClient::with_cancel(StubResult::NotFound);
        let args = Cancel {
            json: false,
            id: id(42),
        };

        let (subscriber, writer) = test_subscriber_message_only();
        let err = async { args.handle(&client).await.unwrap_err() }
            .with_subscriber(subscriber)
            .await;

        assert!(err.is::<Exit>(), "expected an Exit error, got {err:?}");
        assert!(
            client.cancel_was_called(),
            "the DELETE should have been issued"
        );
        assert_eq!(writer.to_string(), indoc! {"
            ✘ ERROR: No Flox Factory build found with ID 42.
            Use 'flox factory list' to see existing builds.
        "});
    }

    #[tokio::test]
    async fn delete_auth_rejected_returns_exit_5() {
        // Auth rejected on the DELETE is never retryable: exit 5.
        let client = StubFactoryClient::with_cancel(StubResult::AuthRejected);
        let args = Cancel {
            json: false,
            id: id(42),
        };

        let (subscriber, writer) = test_subscriber_message_only();
        let err = async { args.handle(&client).await.unwrap_err() }
            .with_subscriber(subscriber)
            .await;

        assert!(err.is::<Exit>(), "expected an Exit error, got {err:?}");
        assert_eq!(writer.to_string(), indoc! {"
            ✘ ERROR: Authentication was rejected by the Flox Factory.
            Run 'flox auth login' and try again.
        "});
    }

    #[tokio::test]
    async fn id_beyond_i64_max_is_not_found_without_a_request() {
        // An ID above i64::MAX cannot name a real build. The stub would answer
        // the DELETE successfully, so reaching the not-found path at all proves
        // the range guard fired before any request — and no DELETE was issued.
        let client = StubFactoryClient::with_cancel(StubResult::Build("cancelled".to_string()));
        let args = Cancel {
            json: false,
            id: NonZeroU64::new(u64::MAX).unwrap(),
        };

        let (subscriber, writer) = test_subscriber_message_only();
        let err = async { args.handle(&client).await.unwrap_err() }
            .with_subscriber(subscriber)
            .await;

        assert!(err.is::<Exit>(), "expected an Exit error, got {err:?}");
        assert!(
            !client.cancel_was_called(),
            "no DELETE should be issued for an out-of-range ID"
        );
        assert_eq!(writer.to_string(), indoc! {"
            ✘ ERROR: No Flox Factory build found with ID 18446744073709551615.
            Use 'flox factory list' to see existing builds.
        "});
    }
}
