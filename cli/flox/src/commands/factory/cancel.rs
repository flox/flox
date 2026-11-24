use anyhow::Result;
use bpaf::Bpaf;
use floxhub_client::{
    EffectiveFactoryRunStatus,
    FactoryClientError,
    FactoryClientTrait,
    FactoryRunId,
    FactoryRunResponse,
};
use indoc::{formatdoc, indoc};
use tracing::instrument;

use crate::utils::message;
use crate::{Exit, subcommand_metric};

/// Cancel a single Flox Factory run.
///
/// Exit codes: 0 success, 1 service error, 2 not found, 4 unreachable,
/// 5 authentication rejected.
#[derive(Debug, Clone, PartialEq, Bpaf)]
pub struct Cancel {
    /// Display output as JSON
    #[bpaf(long)]
    pub json: bool,

    /// ID of the Flox Factory run to cancel
    #[bpaf(positional("ID"))]
    pub id: FactoryRunId,
}

impl Cancel {
    #[instrument(name = "cancel", skip_all)]
    pub async fn handle(self, client: &impl FactoryClientTrait) -> Result<()> {
        subcommand_metric!("factory::cancel");

        // Issue the idempotent cancel. The body of a 200 has the run's
        // effective status, from which the outcome (initiated vs already
        // terminal) is read; every HTTP and transport failure is classified in the client
        // layer into the typed error the exit-code contract maps.
        match client.cancel_factory_run(self.id).await {
            Ok(run) => match success_outcome(&run.status, self.id) {
                // Terminal: the cancel is fully resolved. Print to stdout, exit 0.
                CancelOutcome::Resolved(message) => {
                    print!("{}", render(&run, &message, self.json)?);
                    Ok(())
                },
                // Accepted but not yet terminal: the idempotent DELETE succeeded,
                // so exit 0 (automation must not retry). Warn about the caveat,
                // and still emit the body under `--json`.
                CancelOutcome::Accepted(message) => {
                    if self.json {
                        print!("{}", render(&run, &message, true)?);
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
fn classify_error(err: &FactoryClientError, id: FactoryRunId) -> (String, u8) {
    match err {
        // No such run: exit 2.
        FactoryClientError::NotFound => (
            formatdoc! {"
                No Flox Factory run found with ID {id}.
                Use 'flox factory list' to see existing runs."},
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
        // A 409 means the coordinator has not received the run yet.
        FactoryClientError::Server(api)
            if api.status().is_some_and(|status| status.as_u16() == 409) =>
        {
            (
                formatdoc! {"
                    The Flox Factory is still dispatching run {id}.
                    Try 'flox factory cancel {id}' again in a moment."},
                1,
            )
        },
        // A 502 can also mean the coordinator lost the run. Do not promise
        // that retry will resolve every service failure.
        FactoryClientError::Server(_) => (
            formatdoc! {"
                The Flox Factory could not confirm cancellation for run {id}.
                Try again in a moment.
                If the problem persists, use 'flox factory status {id}' to check its state."},
            1,
        ),
        // An unrecognized response has no known recovery action. Exit 1.
        FactoryClientError::APIError(_) => (
            formatdoc! {"
                The Flox Factory could not cancel run {id}.
                Use 'flox factory status {id}' to check its state."},
            1,
        ),
    }
}

/// The outcome of a successful (HTTP 200) cancel response, read from the
/// run's effective `status`.
#[derive(Debug, PartialEq)]
enum CancelOutcome {
    /// The run reached a terminal state; the cancel is fully resolved.
    /// Printed to stdout, exit 0.
    Resolved(String),
    /// The cancel was accepted but the run has not yet reached a terminal
    /// state. Surfaced as a warning, exit 0: the idempotent DELETE succeeded,
    /// so automation must not retry it.
    Accepted(String),
    /// A 200 whose status is outside the lifecycle vocabulary: a contract
    /// violation, not a successful cancel. Surfaced as an error, exit 1.
    Unexpected(String),
}

/// Classify a successful (HTTP 200) cancel response from the run's effective
/// `status`.
///
/// Matching the typed [`EffectiveFactoryRunStatus`] exhaustively makes a status
/// newly added to the enum a compile error here rather than a silent
/// fall-through; until the enum names it, a new server status arrives as
/// `Unknown`.
///
/// A satisfied cancel reports a terminal `cancelled`/`completed`/`failed`/
/// `timed_out`. A non-terminal `running`/`pending` means the cancel was accepted
/// but the run has not yet stopped; like the read verbs, that is a normal
/// outcome, not a retryable error. Only an unrecognized status is a contract
/// violation.
fn success_outcome(status: &EffectiveFactoryRunStatus, id: FactoryRunId) -> CancelOutcome {
    let word = status.as_str();
    match status {
        EffectiveFactoryRunStatus::Cancelled => {
            CancelOutcome::Resolved(format!("Cancellation initiated for Flox Factory run {id}."))
        },
        EffectiveFactoryRunStatus::Completed => CancelOutcome::Resolved(format!(
            "Flox Factory run {id} already completed; nothing to cancel."
        )),
        EffectiveFactoryRunStatus::Failed => CancelOutcome::Resolved(format!(
            "Flox Factory run {id} already failed; nothing to cancel."
        )),
        EffectiveFactoryRunStatus::TimedOut => CancelOutcome::Resolved(format!(
            "Flox Factory run {id} already timed out; nothing to cancel."
        )),
        EffectiveFactoryRunStatus::Running | EffectiveFactoryRunStatus::Pending => {
            CancelOutcome::Accepted(accepted_message(id, word))
        },
        // A status outside the known vocabulary is a contract violation, not a
        // successful cancel.
        EffectiveFactoryRunStatus::Unknown(_) => CancelOutcome::Unexpected(formatdoc! {"
            The Flox Factory returned an unexpected status '{word}' for run {id}.
            Use 'flox factory status {id}' to check the run."}),
    }
}

/// The "accepted but not yet terminal" message for a non-terminal status.
fn accepted_message(id: FactoryRunId, status: &str) -> String {
    formatdoc! {"
        Cancellation accepted for Flox Factory run {id}.
        The run's status is still {status}.
        Use 'flox factory status {id}' to follow it."}
}

/// Render a successful cancel: the human outcome line, or the raw run as JSON
/// when `--json` is set (mirrors `status.rs`'s split).
fn render(run: &FactoryRunResponse, outcome: &str, json: bool) -> Result<String> {
    if json {
        Ok(format!("{}\n", serde_json::to_string_pretty(run)?))
    } else {
        Ok(format!("{outcome}\n"))
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use floxhub_client::FactoryApiError;
    use pretty_assertions::assert_eq;
    use tracing::instrument::WithSubscriber;

    use super::*;
    use crate::commands::factory::test_helpers::{StubFactoryClient, StubResult, make_run};

    fn id(n: i64) -> FactoryRunId {
        n.to_string().parse().unwrap()
    }

    fn server_error(status: u16) -> FactoryClientError {
        let response = http::Response::builder()
            .status(status)
            .body(String::new())
            .unwrap()
            .into();
        FactoryClientError::Server(FactoryApiError::UnexpectedResponse(response))
    }

    // -------------------------------------------------------------------------
    // success_outcome: the status-to-outcome mapping
    // -------------------------------------------------------------------------

    #[test]
    fn cancelled_status_is_resolved() {
        assert_eq!(
            success_outcome(&EffectiveFactoryRunStatus::Cancelled, id(42)),
            CancelOutcome::Resolved("Cancellation initiated for Flox Factory run 42.".to_string())
        );
    }

    #[test]
    fn completed_status_is_resolved() {
        assert_eq!(
            success_outcome(&EffectiveFactoryRunStatus::Completed, id(42)),
            CancelOutcome::Resolved(
                "Flox Factory run 42 already completed; nothing to cancel.".to_string()
            )
        );
    }

    #[test]
    fn failed_status_is_resolved() {
        assert_eq!(
            success_outcome(&EffectiveFactoryRunStatus::Failed, id(42)),
            CancelOutcome::Resolved(
                "Flox Factory run 42 already failed; nothing to cancel.".to_string()
            )
        );
    }

    #[test]
    fn timed_out_status_is_resolved() {
        // `timed_out` is terminal, so it is an "already terminal" exit-0
        // outcome, not a contract violation.
        assert_eq!(
            success_outcome(&EffectiveFactoryRunStatus::TimedOut, id(42)),
            CancelOutcome::Resolved(
                "Flox Factory run 42 already timed out; nothing to cancel.".to_string()
            )
        );
    }

    #[test]
    fn non_terminal_status_is_accepted() {
        // A 200 with a non-terminal status means the cancel was accepted but the
        // run has not yet stopped: a normal exit-0 outcome, not a retry.
        assert_eq!(
            success_outcome(&EffectiveFactoryRunStatus::Running, id(42)),
            CancelOutcome::Accepted(
                indoc! {"
                    Cancellation accepted for Flox Factory run 42.
                    The run's status is still running.
                    Use 'flox factory status 42' to follow it."}
                .to_string()
            )
        );
    }

    #[test]
    fn pending_status_is_accepted() {
        // `pending` is an undispatched run's effective status: a known
        // non-terminal outcome, so the cancel is accepted (exit 0) rather than
        // read as a contract violation.
        assert_eq!(
            success_outcome(&EffectiveFactoryRunStatus::Pending, id(42)),
            CancelOutcome::Accepted(
                indoc! {"
                    Cancellation accepted for Flox Factory run 42.
                    The run's status is still pending.
                    Use 'flox factory status 42' to follow it."}
                .to_string()
            )
        );
    }

    #[test]
    fn out_of_vocabulary_status_is_unexpected() {
        // Only a status outside the known vocabulary is a contract violation.
        assert_eq!(
            success_outcome(
                &EffectiveFactoryRunStatus::Unknown("frobnicated".to_string()),
                id(42)
            ),
            CancelOutcome::Unexpected(
                indoc! {"
                    The Flox Factory returned an unexpected status 'frobnicated' for run 42.
                    Use 'flox factory status 42' to check the run."}
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
                    No Flox Factory run found with ID 7.
                    Use 'flox factory list' to see existing runs."}
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
    fn dispatch_in_flight_is_retryable_exit_1() {
        assert_eq!(
            classify_error(&server_error(409), id(7)),
            (
                indoc! {"
                    The Flox Factory is still dispatching run 7.
                    Try 'flox factory cancel 7' again in a moment."}
                .to_string(),
                1,
            )
        );
    }

    #[test]
    fn service_failure_is_exit_1_without_promising_recovery() {
        assert_eq!(
            classify_error(&server_error(502), id(7)),
            (
                indoc! {"
                    The Flox Factory could not confirm cancellation for run 7.
                    Try again in a moment.
                    If the problem persists, use 'flox factory status 7' to check its state."}
                .to_string(),
                1,
            )
        );
    }

    #[test]
    fn unrecognised_response_degrades_to_service_error_exit_1() {
        // A non-auth 4xx, or a 200 whose body did not parse as a run, is
        // classified as the generic API error and degrades to a retryable
        // service error rather than escalating to a bespoke outcome.
        let err = FactoryClientError::APIError(FactoryApiError::InvalidRequest("boom".to_string()));
        assert_eq!(
            classify_error(&err, id(7)),
            (
                indoc! {"
                    The Flox Factory could not cancel run 7.
                    Use 'flox factory status 7' to check its state."}
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
        let run = make_run(
            42,
            "x86_64-linux",
            "hello",
            Some(EffectiveFactoryRunStatus::Running),
        );
        assert_eq!(
            render(
                &run,
                "Cancellation initiated for Flox Factory run 42.",
                false
            )
            .unwrap(),
            "Cancellation initiated for Flox Factory run 42.\n"
        );
    }

    #[test]
    fn render_json_emits_the_run_and_ignores_the_outcome() {
        let run = make_run(
            42,
            "x86_64-linux",
            "hello",
            Some(EffectiveFactoryRunStatus::Running),
        );
        let rendered = render(&run, "OUTCOME LINE", true).unwrap();
        // The JSON branch serializes the run and drops the human outcome
        // line. Assert the behavior of the branch, not serde's round-trip
        // fidelity (AGENTS.md).
        assert!(!rendered.contains("OUTCOME LINE"));
        assert!(rendered.contains("\"run_id\": 42"));
        assert!(rendered.contains("\"status\": \"running\""));
    }

    // -------------------------------------------------------------------------
    // handle — wiring the DELETE outcome to the exit-code contract
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn successful_cancel_issues_delete_and_returns_ok() {
        let client =
            StubFactoryClient::with_cancel(StubResult::Run(EffectiveFactoryRunStatus::Cancelled));
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
            ✘ ERROR: The Flox Factory could not confirm cancellation for run 42.
            Try again in a moment.
            If the problem persists, use 'flox factory status 42' to check its state.
        "});
    }

    #[tokio::test]
    async fn delete_non_terminal_status_warns_and_exits_zero() {
        // A 200 with a non-terminal status means the cancel was accepted but the
        // run has not yet stopped: a normal success (exit 0) with a warning,
        // not a retryable error.
        let client =
            StubFactoryClient::with_cancel(StubResult::Run(EffectiveFactoryRunStatus::Running));
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
            ! Cancellation accepted for Flox Factory run 42.
            The run's status is still running.
            Use 'flox factory status 42' to follow it.
        "});
    }

    #[tokio::test]
    async fn delete_out_of_vocabulary_status_is_a_service_error() {
        // A 200 whose status is outside the lifecycle vocabulary is a contract
        // violation, not a successful cancel: the verb returns exit 1 and emits
        // the error to stderr rather than printing the run to stdout.
        let client = StubFactoryClient::with_cancel(StubResult::Run(
            EffectiveFactoryRunStatus::Unknown("frobnicated".to_string()),
        ));
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
            ✘ ERROR: The Flox Factory returned an unexpected status 'frobnicated' for run 42.
            Use 'flox factory status 42' to check the run.
        "});
    }

    #[tokio::test]
    async fn delete_not_found_returns_exit_2() {
        // A DELETE against a missing run 404s; the client maps it to NotFound
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
            ✘ ERROR: No Flox Factory run found with ID 42.
            Use 'flox factory list' to see existing runs.
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
}
