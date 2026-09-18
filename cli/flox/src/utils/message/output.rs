//! CLI output, independent of tracing subscribers and diagnostic filters.

use std::fmt::{self, Display};
use std::future::Future;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, OnceLock};

tokio::task_local! {
    static OUTPUT: Output;
}

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

    /// Inject output for synchronous work, restoring the enclosing context on exit.
    pub(crate) fn sync_scope<T>(&self, f: impl FnOnce() -> T) -> T {
        OUTPUT.sync_scope(self.clone(), f)
    }

    /// Spawned tasks must receive a clone explicitly; scopes do not propagate on spawn.
    pub(crate) fn scope<F: Future>(
        &self,
        future: F,
    ) -> tokio::task::futures::TaskLocalFuture<Output, F> {
        OUTPUT.scope(self.clone(), future)
    }
}

/// Compatibility context for the existing free `message::` helpers.
/// Tests inject per-task writers rather than replacing a process-wide writer.
pub(crate) fn current_output() -> Output {
    OUTPUT.try_with(Clone::clone).unwrap_or_else(|_| {
        DEFAULT_OUTPUT
            .get()
            .cloned()
            .unwrap_or_else(|| Output::new(io::stdout(), io::stderr()))
    })
}

/// Install only the production output. Tests override it with isolated scopes.
pub(crate) fn set_default_output(output: Output) {
    let _ = DEFAULT_OUTPUT.set(output);
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use flox_rust_sdk::utils::logging::test_helpers::CollectingWriter;

    use super::*;

    #[derive(Clone, Debug)]
    struct Capture(CollectingWriter);

    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            (&self.0).write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            (&self.0).flush()
        }
    }

    pub(crate) fn capture_output() -> (Output, CollectingWriter, CollectingWriter) {
        let stdout = CollectingWriter::default();
        let stderr = CollectingWriter::default();
        let output = Output::new(Capture(stdout.clone()), Capture(stderr.clone()));
        (output, stdout, stderr)
    }

    pub(crate) fn capture_messages() -> (Output, CollectingWriter) {
        let (output, _, stderr) = capture_output();
        (output, stderr)
    }

    pub(crate) trait WithOutput: Future + Sized {
        fn with_output(
            self,
            output: Output,
        ) -> tokio::task::futures::TaskLocalFuture<Output, Self> {
            output.scope(self)
        }
    }

    impl<F: Future> WithOutput for F {}
}

#[cfg(test)]
mod tests {
    use super::test_helpers::capture_output;
    use super::*;
    use crate::utils::message;

    #[test]
    fn nested_output_scopes_restore_the_parent_on_unwind() {
        let (outer, outer_stdout, outer_stderr) = capture_output();
        let (inner, inner_stdout, inner_stderr) = capture_output();
        outer.sync_scope(|| {
            message::plain("before");
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                inner.sync_scope(|| {
                    message::plain("inner");
                    panic!("leave the nested scope");
                });
            }));
            message::plain("after");
        });
        assert_eq!(
            (
                outer_stdout.to_string(),
                outer_stderr.to_string(),
                inner_stdout.to_string(),
                inner_stderr.to_string()
            ),
            (
                String::new(),
                "before\nafter\n".to_string(),
                String::new(),
                "inner\n".to_string()
            ),
        );
    }

    #[test]
    fn quiet_suppresses_notices_but_preserves_errors_and_results() {
        let (output, stdout, stderr) = capture_output();
        let output = output.with_quiet(true);
        output.sync_scope(|| {
            message::plain("notice");
            message::warning("warning");
            message::updated("success");
            message::error("first line\nsecond line");
            output.stdout("result").unwrap();
        });
        assert_eq!(
            (stdout.to_string(), stderr.to_string()),
            (
                "result".to_string(),
                "✘ ERROR: first line\nsecond line\n".to_string()
            ),
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
    fn messages_remain_best_effort_and_result_writes_return_errors() {
        let output = Output::new(BrokenPipe, BrokenPipe);
        output.sync_scope(|| {
            message::plain("notice");
            message::error("failure");
        });
        assert_eq!(
            output.stdout("result").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocking_work_threads_and_callbacks_keep_injected_output() {
        let (output, stdout, stderr) = capture_output();
        let callback_output = output.clone();
        let callback = move || callback_output.sync_scope(|| message::plain("callback"));
        let blocking_output = output.clone();
        tokio::task::spawn_blocking(move || {
            blocking_output.sync_scope(|| message::plain("blocking"));
            callback();
        })
        .await
        .unwrap();
        std::thread::spawn(move || output.sync_scope(|| message::error("thread")))
            .join()
            .unwrap();
        assert_eq!(
            (stdout.to_string(), stderr.to_string()),
            (
                String::new(),
                "blocking\ncallback\n✘ ERROR: thread\n".to_string()
            ),
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawned_tasks_keep_injected_output_isolated() {
        let (first, first_stdout, first_stderr) = capture_output();
        let (second, second_stdout, second_stderr) = capture_output();
        let task = |output: Output, label: &'static str| {
            tokio::spawn(output.scope(async move {
                for _ in 0..10 {
                    tokio::task::yield_now().await;
                    message::plain(label);
                    current_output().stdout(label).unwrap();
                }
            }))
        };
        let (one, two) = tokio::join!(task(first, "first"), task(second, "second"));
        one.unwrap();
        two.unwrap();
        assert_eq!(
            (
                first_stdout.to_string(),
                first_stderr.to_string(),
                second_stdout.to_string(),
                second_stderr.to_string()
            ),
            (
                "first".repeat(10),
                "first\n".repeat(10),
                "second".repeat(10),
                "second\n".repeat(10)
            ),
        );
    }
}
