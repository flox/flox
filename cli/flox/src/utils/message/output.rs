//! CLI output, independent of tracing subscribers and diagnostic filters.

use std::fmt::{self, Display};
use std::io::{self, Write};
use std::sync::{Arc, Mutex, OnceLock};

static DEFAULT_OUTPUT: OnceLock<Output> = OnceLock::new();

/// Writers shared by one CLI invocation. Clones share each stream's write lock.
#[derive(Clone)]
pub(crate) struct Output {
    quiet: bool,
    stdout: Arc<Mutex<Box<dyn Write + Send>>>,
    stderr: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl fmt::Debug for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Output").finish_non_exhaustive()
    }
}

impl Output {
    pub(crate) fn new(
        stdout: impl Write + Send + 'static,
        stderr: impl Write + Send + 'static,
    ) -> Self {
        Self {
            quiet: false,
            stdout: Arc::new(Mutex::new(Box::new(stdout))),
            stderr: Arc::new(Mutex::new(Box::new(stderr))),
        }
    }

    pub(crate) fn with_quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    pub(crate) fn notice(&self, value: impl Display) -> io::Result<()> {
        if self.quiet {
            return Ok(());
        }
        self.stderr(value)
    }

    /// Command results belong on stdout; notices and errors belong on stderr.
    pub(crate) fn stdout(&self, value: impl Display) -> io::Result<()> {
        write!(self.stdout.lock().unwrap(), "{value}")
    }

    pub(crate) fn stderr(&self, value: impl Display) -> io::Result<()> {
        writeln!(self.stderr.lock().unwrap(), "{value}")
    }
}

/// Production callers share the initialized terminal writers. Unit tests use
/// tracing capture unless a subprocess probe explicitly initializes real output.
pub(crate) fn current_output() -> Output {
    DEFAULT_OUTPUT.get().cloned().unwrap_or_else(|| {
        #[cfg(not(test))]
        {
            Output::new(io::stdout(), io::stderr())
        }
        #[cfg(test)]
        {
            super::test_helpers::tracing_output()
        }
    })
}

/// Install the production output once, including its progress and quiet policy.
pub(crate) fn set_default_output(output: Output) {
    let _ = DEFAULT_OUTPUT.set(output);
}

#[cfg(test)]
mod tests {
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::prelude::*;

    use super::*;
    use crate::utils::message;
    use crate::utils::message::test_helpers::{capture_output, tracing_output};

    #[test]
    fn quiet_suppresses_notices_but_preserves_errors_and_results() {
        let (capture, stdout, stderr) = capture_output();
        tracing::subscriber::with_default(tracing_subscriber::registry().with(capture), || {
            let output = tracing_output().with_quiet(true);
            output.notice("notice").unwrap();
            output.stderr("error\ncontinued").unwrap();
            output.stdout("result").unwrap();
        });
        assert_eq!(
            (stdout.to_string(), stderr.to_string()),
            ("result".to_string(), "error\ncontinued\n".to_string()),
        );
    }

    #[derive(Debug)]
    struct BrokenPipe;

    impl Write for BrokenPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn writer_failures_are_returned_to_the_caller() {
        let output = Output::new(BrokenPipe, BrokenPipe);
        assert_eq!(
            (
                output.notice("notice").unwrap_err().kind(),
                output.stderr("error").unwrap_err().kind(),
                output.stdout("result").unwrap_err().kind(),
            ),
            (
                io::ErrorKind::BrokenPipe,
                io::ErrorKind::BrokenPipe,
                io::ErrorKind::BrokenPipe
            ),
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscriber_capture_preserves_streams_and_isolates_concurrent_work() {
        let (first, first_stdout, first_stderr) = capture_output();
        let (second, second_stdout, second_stderr) = capture_output();
        let task = |label: &'static str| async move {
            let callback = move || message::plain(format!("{label}\ncontinued"));
            // Exercise the normal tracing propagation mechanism used by tests.
            tokio::spawn(
                async move {
                    for _ in 0..3 {
                        tokio::task::yield_now().await;
                        callback();
                        current_output().stdout(format_args!("{label}✓")).unwrap();
                        tracing::warn!("diagnostic excluded from message capture");
                    }
                }
                .with_current_subscriber(),
            )
            .await
            .unwrap();
        };
        tokio::join!(
            task("first").with_subscriber(tracing_subscriber::registry().with(first)),
            task("second").with_subscriber(tracing_subscriber::registry().with(second)),
        );
        assert_eq!(
            (
                first_stdout.to_string(),
                first_stderr.to_string(),
                second_stdout.to_string(),
                second_stderr.to_string()
            ),
            (
                "first✓".repeat(3),
                "first\ncontinued\n".repeat(3),
                "second✓".repeat(3),
                "second\ncontinued\n".repeat(3)
            ),
        );
    }
}
