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
