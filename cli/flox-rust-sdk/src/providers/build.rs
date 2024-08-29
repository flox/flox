use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::Receiver;
use std::sync::LazyLock;
use std::thread;

use thiserror::Error;
use tracing::{debug, warn};

use crate::utils::CommandExt;

static FLOX_BUILD_MK: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("FLOX_BUILD_MK")
        .unwrap_or_else(|_| env!("FLOX_BUILD_MK").to_string())
        .into()
});

static GNUMAKE_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("GNUMAKE_BIN")
        .unwrap_or_else(|_| env!("GNUMAKE_BIN").to_string())
        .into()
});

pub trait ManifestBuilder {
    /// Build the specified packages defined in the environment at `flox_env`.
    /// The build process will start in the background.
    /// To process the output, the caller should iterate over the returned [BuildOutput].
    /// Once the process is complete, the [BuildOutput] will yield an [Output::Exit] message.
    fn build(
        &self,
        base_dir: &Path,
        flox_env: &Path,
        package: &[String],
    ) -> Result<BuildOutput, ManifestBuilderError>;
}

#[derive(Debug, Error)]
pub enum ManifestBuilderError {
    #[error("failed to call package builder: {0}")]
    CallBuilderError(#[source] std::io::Error),
}

pub enum Output {
    /// A line of stdout output from the build process.
    Stdout(String),
    /// A line of stderr output from the build process.
    Stderr(String),
    /// The build process has exited with the given status.
    Exit(ExitStatus),
}

/// Output received from an ongoing build process.
/// Callers of [ManifestBuilder::build] should iterate over this type
/// to process the output of the build process and await its completion.
#[must_use = "The build process is started in the background.
To process the output and wait for the process to finish,
iterate over the returned BuildOutput."]
pub struct BuildOutput {
    receiver: Receiver<Output>,
}

impl Iterator for BuildOutput {
    type Item = Output;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

/// A manifest builder that uses the [FLOX_BUILD_MK] makefile to build packages.
pub struct FloxBuildMk;

impl ManifestBuilder for FloxBuildMk {
    /// Build `packages` defined in the environment rendered at
    /// `flox_env` using the [FLOX_BUILD_MK] makefile.
    ///
    /// `packages` SHOULD be a list of package names defined in the
    /// environment or an empty list to build all packages.
    ///
    /// If a package is not found in the environment,
    /// the makefile will fail with an error.
    /// However, currently the caller doesn't distinguish different error cases.
    ///
    /// The makefile is executed with its current working directory set to `base_dir`.
    /// Upon success, the builder will have built the specified packages
    /// and created links to the respective store paths in `base_dir/result-<build name>`.
    ///
    /// The build process will start in the background.
    /// To process the output, the caller should iterate over the returned [BuildOutput].
    /// Once the process is complete, the [BuildOutput] will yield an [Output::Exit] message.
    fn build(
        &self,
        base_dir: &Path,
        flox_env: &Path,
        packages: &[String],
    ) -> Result<BuildOutput, ManifestBuilderError> {
        let mut command = Command::new(&*GNUMAKE_BIN);
        command.arg("-f").arg(&*FLOX_BUILD_MK);
        command.arg("-C").arg(base_dir);
        command.arg(format!("FLOX_ENV={}", flox_env.display()));

        // todo: extra makeflags, eventually

        // add packages
        command.args(packages);

        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        debug!("running manifest build builder: {}", command.display());
        let mut child = command
            .spawn()
            .map_err(ManifestBuilderError::CallBuilderError)?;

        let (sender, receiver) = std::sync::mpsc::channel();
        let stdout_sender = sender.clone();
        let stderr_sender = sender.clone();
        let command_status_sender = sender;

        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            let stdout = std::io::BufReader::new(stdout);
            read_output_to_channel(stdout, stdout_sender, Output::Stdout);
        });

        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            let stderr = std::io::BufReader::new(stderr);
            read_output_to_channel(stderr, stderr_sender, Output::Stderr);
        });

        thread::spawn(move || {
            let status = child.wait().expect("failed to wait on child");
            let _ = command_status_sender.send(Output::Exit(status));
        });

        Ok(BuildOutput { receiver })
    }
}

/// Read output from a reader and send it to a channel
/// until the reader is exhausted or the receiver is dropped.
fn read_output_to_channel(
    reader: impl BufRead,
    sender: std::sync::mpsc::Sender<Output>,
    mk_output: impl Fn(String) -> Output,
) {
    for line in reader.lines() {
        let line = match line {
            Err(e) => {
                warn!("failed to read line: {e}");
                continue;
            },
            Ok(line) => line,
        };

        let Ok(_) = sender.send(mk_output(line)) else {
            // if the receiver is dropped, we can stop reading
            break;
        };
    }
}
