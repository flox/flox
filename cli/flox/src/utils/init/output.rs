//! CLI startup owns terminal output, progress coordination, and subscriber installation.

use std::sync::OnceLock;

use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::reload::Handle;
use tracing_subscriber::{EnvFilter, Registry};

use super::{logger, progress};
use crate::commands::Verbosity;
use crate::utils::message::{self, Output};
use crate::utils::metrics::MetricsLayer;

struct CliOutput {
    filter: Handle<EnvFilter, Registry>,
    output: Output,
}

static CLI_OUTPUT: OnceLock<CliOutput> = OnceLock::new();

pub(crate) fn init_output(verbosity: Option<Verbosity>) -> Output {
    let verbosity = verbosity.unwrap_or_default();
    let cli = CLI_OUTPUT.get_or_init(|| {
        let (subscriber, filter, output) = create_registry_and_filter_reload_handle();
        let output = output.with_quiet(matches!(verbosity, Verbosity::Quiet));
        subscriber.init();
        message::set_default_output(output.clone());
        CliOutput { filter, output }
    });
    if let Err(err) = logger::update_filters(&cli.filter, logger::verbosity_filter(verbosity)) {
        tracing::error!(error = %err, "Updating logger filter failed");
        message::error(format!("Updating logger filter failed: {err}"));
    }
    cli.output.clone()
}

pub(crate) fn create_registry_and_filter_reload_handle() -> (
    impl tracing_subscriber::layer::SubscriberExt,
    Handle<EnvFilter, Registry>,
    Output,
) {
    create_registry_with_console_writer(None)
}

fn create_registry_with_console_writer(
    console_writer: Option<BoxMakeWriter>,
) -> (
    impl tracing_subscriber::layer::SubscriberExt,
    Handle<EnvFilter, Registry>,
    Output,
) {
    let (progress_layer, writer) = progress::progress_layer();
    let output = Output::new(std::io::stdout(), writer.clone());
    let (log_layer, filter_reload_handle) =
        logger::console_layer(console_writer.unwrap_or_else(|| BoxMakeWriter::new(writer)));
    let metrics_layer = MetricsLayer::new();
    let sentry_layer = logger::sentry_layer();
    // Filtered layer must come first.
    // This appears to be the only way to avoid logs of the `flox_command` trace
    // which is processed by the `log_layer` irrespective of the filter applied to it.
    // My current understanding is, that it because the `metrics_layer` (at least) is
    // registering `Interest` for the event and that somehow bypasses the filter?!
    let registry = tracing_subscriber::registry()
        .with(log_layer)
        .with(progress_layer)
        .with(metrics_layer)
        .with(sentry_layer);

    (registry, filter_reload_handle, output)
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::process::{Command, Stdio};

    use flox_rust_sdk::utils::logging::test_helpers::CollectingWriter;
    use nix::pty::{Winsize, openpty};
    use sentry::test::with_captured_events;

    use super::*;
    use crate::utils::init::logger::{update_filters, verbosity_filter};
    use crate::utils::message::test_helpers::capture_output;
    use crate::utils::tracing::{command_finished, command_started};

    #[test]
    fn console_filters_leave_sentry_and_messages_enabled() {
        temp_env::with_vars(
            [("FLOX_LOG", None::<&str>), ("RUST_LOG", Some("trace"))],
            || {
                for filter in ["off", "trace", "off"] {
                    let console = CollectingWriter::default();
                    let (output, stdout, stderr) = capture_output();
                    let events = with_captured_events(|| {
                        let (subscriber, handle, _) = create_registry_with_console_writer(Some(
                            BoxMakeWriter::new(console.clone()),
                        ));
                        tracing::subscriber::with_default(subscriber, || {
                            update_filters(&handle, filter).unwrap();
                            output.sync_scope(|| {
                                message::plain("user notice");
                                message::warning("user warning");
                                message::error("user error");
                                output.stdout("command result").unwrap();
                            });
                            tracing::trace!("trace diagnostic");
                            tracing::debug!("debug diagnostic");
                            tracing::info!("info diagnostic");
                            tracing::warn!("warn diagnostic");
                            tracing::error!("error diagnostic");
                        });
                    });
                    assert_eq!(stdout.to_string(), "command result");
                    assert_eq!(
                        stderr.to_string(),
                        "user notice\n! user warning\n✘ ERROR: user error\n"
                    );
                    assert_eq!(events.len(), 1);
                    assert_eq!(events[0].level, sentry::Level::Error);
                    assert_eq!(events[0].message.as_deref(), Some("error diagnostic"));
                    assert_eq!(
                        events[0]
                            .breadcrumbs
                            .iter()
                            .map(|b| (b.level, b.message.as_deref()))
                            .collect::<Vec<_>>(),
                        vec![
                            (sentry::Level::Info, Some("info diagnostic")),
                            (sentry::Level::Warning, Some("warn diagnostic"))
                        ],
                    );
                    if filter == "off" {
                        assert_eq!(console.to_string(), "");
                    } else {
                        let diagnostic = console.to_string();
                        for level in ["trace", "debug", "info", "warn", "error"] {
                            assert!(
                                diagnostic.contains(&format!("{level} diagnostic")),
                                "{diagnostic}"
                            );
                        }
                        assert!(!diagnostic.contains("user notice"));
                    }
                }
            },
        );
    }

    #[test]
    fn command_context_reaches_sentry_without_ui_or_duplicate_error_events() {
        temp_env::with_var("FLOX_LOG", Some("off"), || {
            let (output, _, stderr) = capture_output();
            let console = CollectingWriter::default();
            let events = with_captured_events(|| {
                let (subscriber, handle, _) =
                    create_registry_with_console_writer(Some(BoxMakeWriter::new(console.clone())));
                tracing::subscriber::with_default(subscriber, || {
                    update_filters(&handle, "off").unwrap();
                    output.with_quiet(true).sync_scope(|| {
                        command_started("install");
                        message::updated("presentation only");
                        let error =
                            anyhow::anyhow!("request failed").context("installation failed");
                        command_finished("install", 1, Some("uncategorized"), Some(&error));
                        tracing::error!("later operational failure");
                    });
                });
            });
            assert_eq!(
                (console.to_string(), stderr.to_string()),
                (String::new(), String::new())
            );
            assert_eq!(events.len(), 1);
            let breadcrumbs = &events[0].breadcrumbs;
            assert_eq!(
                breadcrumbs
                    .iter()
                    .map(|b| b.message.as_deref())
                    .collect::<Vec<_>>(),
                vec![Some("Command started"), Some("Command finished")]
            );
            assert_eq!(
                breadcrumbs[1].data.get("command"),
                Some(&serde_json::json!("install"))
            );
            assert_eq!(
                breadcrumbs[1].data.get("error"),
                Some(&serde_json::json!("installation failed: request failed"))
            );
        });
    }

    #[test]
    fn flox_log_overrides_verbosity_and_supports_reload() {
        temp_env::with_var("FLOX_LOG", Some("off"), || {
            let console = CollectingWriter::default();
            let (subscriber, handle, _) =
                create_registry_with_console_writer(Some(BoxMakeWriter::new(console.clone())));
            tracing::subscriber::with_default(subscriber, || {
                update_filters(&handle, "trace").unwrap();
                tracing::error!("hidden diagnostic");
                assert_eq!(console.to_string(), "");
                temp_env::with_var("FLOX_LOG", Some("off,flox=debug"), || {
                    update_filters(&handle, "off").unwrap();
                    tracing::debug!("visible diagnostic");
                    tracing::warn!(target: "dependency", "hidden dependency diagnostic");
                });
                update_filters(&handle, "trace").unwrap();
                tracing::error!("hidden after reload");
            });
            let diagnostic = console.to_string();
            assert!(diagnostic.contains("visible diagnostic"), "{diagnostic}");
            assert!(!diagnostic.contains("hidden"), "{diagnostic}");
        });
    }

    // Runs in an isolated process because logger and log-bridge initialization are global.
    #[test]
    fn logger_subprocess_probe() {
        let Ok(setting) = std::env::var("FLOX_TEST_LOGGER_VERBOSITY") else {
            return;
        };
        let verbosity = if setting == "quiet" {
            Verbosity::Quiet
        } else {
            Verbosity::Verbose(setting.parse().unwrap())
        };
        let events = with_captured_events(|| {
            let output = init_output(Some(verbosity));
            std::thread::spawn(|| {
                message::plain("unscoped thread notice");
                message::error("unscoped thread error");
            })
            .join()
            .unwrap();
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                tokio::spawn(async {
                    message::error("unscoped task error");
                })
                .await
                .unwrap();
                tokio::task::spawn_blocking(|| message::error("unscoped blocking error"))
                    .await
                    .unwrap();
            });
            output.sync_scope(|| {
                message::plain("visible user notice");
                message::error("visible user error");
                output.stdout("visible command result\n").unwrap();
                tracing::info!("probe info diagnostic");
                tracing::warn!("probe warn diagnostic");
                tracing::error!("probe error diagnostic");
                log::warn!(target: "test_dependency", "probe dependency warning");
                log::error!(target: "test_dependency", "probe dependency error");
            });
        });
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].message.as_deref(), Some("probe error diagnostic"));
        assert_eq!(events[1].message.as_deref(), Some("probe dependency error"));
        assert_eq!(events[0].breadcrumbs.len(), 2);
        assert_eq!(events[1].breadcrumbs.len(), 3);
    }

    #[test]
    fn terminal_streams_and_sentry_work_with_global_logger() {
        for (verbosity, env_filter, diagnostics_visible) in [
            ("0", None, false),
            ("quiet", None, false),
            ("quiet", Some("trace"), true),
            ("4", Some("off"), false),
            ("2", None, true),
            ("0", Some("trace"), true),
        ] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "utils::init::output::tests::logger_subprocess_probe",
                    "--nocapture",
                ])
                .env("FLOX_TEST_LOGGER_VERBOSITY", verbosity)
                .env("RUST_LOG", "trace")
                .env("NO_COLOR", "1")
                .env_remove("FLOX_LOG");
            if let Some(filter) = env_filter {
                child.env("FLOX_LOG", filter);
            }
            let result = child.output().unwrap();
            let stdout = String::from_utf8(result.stdout).unwrap();
            let stderr = String::from_utf8(result.stderr).unwrap();
            assert!(result.status.success(), "{stdout}\n{stderr}");
            assert!(stdout.contains("visible command result"), "{stdout}");
            assert!(!stdout.contains("diagnostic"), "{stdout}");
            assert_eq!(
                stderr.contains("visible user notice"),
                verbosity != "quiet",
                "{stderr}"
            );
            assert!(stderr.contains("✘ ERROR: visible user error"), "{stderr}");
            assert_eq!(
                stderr.contains("unscoped thread notice"),
                verbosity != "quiet",
                "{stderr}"
            );
            for boundary in ["thread", "task", "blocking"] {
                assert!(
                    stderr.contains(&format!("✘ ERROR: unscoped {boundary} error")),
                    "{stderr}"
                );
            }
            assert!(!stderr.contains('\u{1b}'), "{stderr}");
            assert_eq!(
                stderr.contains("probe error diagnostic"),
                diagnostics_visible,
                "{stderr}"
            );
            assert_eq!(
                stderr.contains("probe dependency error"),
                diagnostics_visible,
                "{stderr}"
            );
        }
    }

    // Isolated process: the real progress writer and global fallback must share a terminal.
    #[test]
    fn progress_subprocess_probe() {
        if std::env::var_os("FLOX_TEST_PROGRESS").is_none() {
            return;
        }
        init_output(Some(Verbosity::Verbose(1)));
        let span = tracing::info_span!("work", progress = "active progress");
        let entered = span.enter();
        std::thread::sleep(std::time::Duration::from_millis(300));
        message::plain("UI-ACTIVE\nUI-CONTINUATION");
        drop(entered);
        // Exiting a span does not drop its progress bar. This caused the original UI bug.
        message::plain("UI-EXITED");
        message::warning("UI-COLOR");
        std::thread::scope(|threads| {
            for worker in 0..3 {
                threads.spawn(move || {
                    for line in 0..4 {
                        message::plain(format!("UI-WORKER-{worker}-{line}"));
                        tracing::info!(worker, line, "CONCURRENT-DIAGNOSTIC");
                    }
                });
            }
        });
        drop(span);
        message::plain("UI-FINISHED");
    }

    #[test]
    fn progress_and_unscoped_messages_share_a_real_terminal() {
        let pty = openpty(
            Some(&Winsize {
                ws_row: 40,
                ws_col: 160,
                ws_xpixel: 0,
                ws_ypixel: 0,
            }),
            None,
        )
        .unwrap();
        let mut master = std::fs::File::from(pty.master);
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                match master.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => bytes.extend_from_slice(&buffer[..n]),
                    // Linux PTYs report EIO after the last slave closes; macOS reports EOF.
                    Err(err) if err.raw_os_error() == Some(nix::libc::EIO) => break,
                    Err(err) => panic!("reading terminal: {err}"),
                }
            }
            String::from_utf8(bytes).unwrap()
        });
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "utils::init::output::tests::progress_subprocess_probe",
                "--nocapture",
            ])
            .env("FLOX_TEST_PROGRESS", "1")
            .env("TERM", "xterm-256color")
            .env("CLICOLOR_FORCE", "1")
            .env_remove("NO_COLOR")
            .env_remove("FLOX_LOG")
            .stderr(Stdio::from(pty.slave))
            .output()
            .unwrap();
        let terminal = reader.join().unwrap();
        assert!(
            result.status.success(),
            "{terminal}\n{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(terminal.contains("active progress"), "{terminal:?}");
        assert!(
            terminal.contains("\u{1b}[2K"),
            "progress was not cleared: {terminal:?}"
        );
        assert!(
            terminal.contains("UI-ACTIVE\r\nUI-CONTINUATION\r\n"),
            "{terminal:?}"
        );
        assert!(
            terminal.contains("\u{1b}[38;5;11m!\u{1b}[39m UI-COLOR"),
            "{terminal:?}"
        );
        for message in ["UI-EXITED", "UI-FINISHED"] {
            assert_eq!(terminal.matches(message).count(), 1, "{terminal:?}");
        }
        for worker in 0..3 {
            for line in 0..4 {
                let message = format!("UI-WORKER-{worker}-{line}\r\n");
                assert_eq!(terminal.matches(&message).count(), 1, "{terminal:?}");
            }
        }
        assert_eq!(
            terminal.matches("CONCURRENT-DIAGNOSTIC").count(),
            12,
            "{terminal:?}"
        );
    }

    #[test]
    fn default_and_quiet_disable_terminal_diagnostics() {
        assert_eq!(verbosity_filter(Verbosity::default()), "off");
        assert_eq!(verbosity_filter(Verbosity::Quiet), "off");
    }
}
