use std::io::{self, Write};
use std::process::{Command, Stdio};

use thiserror::Error;
use tracing::debug;

use crate::utils::CommandExt;

pub trait ContainerBuilder {
    type Error: std::error::Error;
    fn create_container_source(
        &self,
        name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error>;
}

/// Type representing a container source,
/// i.e. a command that writes a container tarball to stdout.
/// This is typically created by [ContainerBuilder::create_container_source].
#[derive(Debug)]
pub struct ContainerSource {
    source_command: Command,
}

impl ContainerSource {
    pub fn new(source_command: Command) -> Self {
        Self { source_command }
    }

    /// Run the container builder script
    /// and write the container tarball to the given sink
    pub fn stream_container(self, sink: &mut impl Write) -> Result<(), ContainerSourceError> {
        let mut container_source_command = self.source_command;

        // ensure the command writes to stdout
        container_source_command.stdout(Stdio::piped());

        debug!(
            "running container source command: {}",
            container_source_command.display()
        );

        let mut handle = container_source_command
            .spawn()
            .map_err(ContainerSourceError::CallContainerSourceCommand)?;
        let mut stdout = handle.stdout.take().expect("stdout set to piped");

        io::copy(&mut stdout, sink).map_err(ContainerSourceError::StreamContainer)?;

        handle
            .wait()
            .map_err(ContainerSourceError::CallContainerSourceCommand)?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ContainerSourceError {
    #[error("failed to call container source command")]
    CallContainerSourceCommand(#[source] std::io::Error),
    #[error("failed to stream container to sink")]
    StreamContainer(#[source] std::io::Error),
}

#[cfg(test)]
mod container_source_tests {
    use std::fs::{self, File};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use indoc::indoc;
    use tempfile::TempDir;

    use super::*;

    /// OS error 26 is "Text file busy",
    /// which can happen when executing a script
    /// that is has been written to immediately before.
    /// We typically see this in tests, where we write
    /// a new script and immediately execute it.
    /// In production use, this should not happen as the script
    /// will be written by a different process (`nix`).
    ///
    /// <https://github.com/rust-lang/rust/issues/114554>
    const ERR_TEXT_FILE_BUSY: i32 = 26;

    const TEST_BUILDER: &str = indoc! {r#"
        #!/usr/bin/env bash
        echo "hello world"
    "#};

    fn create_test_script() -> (TempDir, PathBuf) {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("flox-test-container-builder");
        std::fs::write(&path, TEST_BUILDER).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (tempdir, path)
    }

    #[test]
    fn test_writes_output_to_writer() {
        let (_tempdir, test_script) = create_test_script();

        let mut buf = Vec::new();

        let mut tries = 0;
        loop {
            if tries >= 3 {
                panic!("Test flaked with 'Text file busy' and can be re-run")
            }
            let container_builder = ContainerSource::new(Command::new(&test_script));
            match container_builder.stream_container(&mut buf) {
                Err(ContainerSourceError::CallContainerSourceCommand(e))
                    if e.raw_os_error() == Some(ERR_TEXT_FILE_BUSY) =>
                {
                    dbg!("Text file busy -- ignored");
                    tries += 1;
                    continue;
                },
                result => break result.unwrap(),
            }
        }
        assert_eq!(buf, b"hello world\n");
    }

    #[test]
    fn test_allows_forwarding_to_file() {
        let (tempdir, test_script) = create_test_script();
        let output_path = tempdir.path().join("output");

        let mut file = File::create(&output_path).unwrap();

        // looping to ignore "Text file busy" errors
        // see the comment on `ERR_TEXT_FILE_BUSY` for more information
        let mut tries = 0;
        loop {
            if tries >= 3 {
                panic!("Test flaked with 'Text file busy' and can be re-run")
            }
            let container_builder = ContainerSource::new(Command::new(&test_script));
            match container_builder.stream_container(&mut file) {
                Err(ContainerSourceError::CallContainerSourceCommand(e))
                    if e.raw_os_error() == Some(ERR_TEXT_FILE_BUSY) =>
                {
                    dbg!("Text file busy -- ignored");
                    tries += 1;
                    continue;
                },
                result => break result.unwrap(),
            }
        }
        drop(file);

        let output = fs::read_to_string(&output_path).unwrap();
        assert_eq!(output, "hello world\n");
    }
}
