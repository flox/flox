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

/// Return the initialized writers, or raw terminal streams before startup.
pub(crate) fn current_output() -> Output {
    DEFAULT_OUTPUT
        .get()
        .cloned()
        .unwrap_or_else(|| Output::new(io::stdout(), io::stderr()))
}

/// Install the production output once, including its progress and quiet policy.
pub(crate) fn set_default_output(output: Output) {
    let _ = DEFAULT_OUTPUT.set(output);
}
