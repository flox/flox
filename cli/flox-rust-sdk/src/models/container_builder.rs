use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use thiserror::Error;

/// Type representing a container builder script,
/// i.e. the output of `pkgdb buildenv --container`
/// ([LockedManifest::build_container](crate::models::lockfile::LockedManifest::build_container)).
///
/// The script is executed with no arguments
/// and writes a container tarball to stdout.
///
/// [ContainerBuilder::stream_container] can be used to write that tarball to a sink.

#[derive(Clone, Debug)]
pub struct ContainerBuilder {
    path: PathBuf,
}

impl ContainerBuilder {
    /// Wrap a container builder script at the given path
    ///
    /// Typically this will be created by [LockedManifest::build_container]
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Run the container builder script
    /// and write the container tarball to the given sink
    pub fn stream_container(&self, mut sink: impl Write) -> Result<(), ContainerBuilderError> {
        let mut container_builder_command = Command::new(&self.path);
        container_builder_command.stdout(Stdio::piped());

        let handle = container_builder_command
            .spawn()
            .map_err(ContainerBuilderError::CallContainerBuilder)?;
        let mut stdout = handle.stdout.expect("stdout set to piped");

        io::copy(&mut stdout, &mut sink).map_err(ContainerBuilderError::StreamContainer)?;

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ContainerBuilderError {
    #[error("failed to call container builder")]
    CallContainerBuilder(#[source] std::io::Error),
    #[error("failed to stream container to sink")]
    StreamContainer(#[source] std::io::Error),
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::os::unix::fs::PermissionsExt;

    use indoc::indoc;
    use tempfile::TempDir;

    use super::*;

    /// OS error 26 is "Text file busy",
    /// which can happen when executing a script
    /// that is has been written to immediately before.
    /// We typically see this in tests, where we write
    /// a new script and immediately execute it.
    /// In production use, this should not happen as the script
    /// will be written by a different process (`pkgdb`).
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
        let container_builder = ContainerBuilder::new(test_script);

        let mut buf = Vec::new();

        let mut tries = 0;
        loop {
            if tries >= 3 {
                panic!("Test flaked with 'Text file busy' and can be re-run")
            }
            match container_builder.stream_container(&mut buf) {
                Err(ContainerBuilderError::CallContainerBuilder(e))
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

        let container_builder = ContainerBuilder::new(test_script);
        let mut file = File::create(&output_path).unwrap();

        // looping to ignore "Text file busy" errors
        // see the comment on `ERR_TEXT_FILE_BUSY` for more information
        let mut tries = 0;
        loop {
            if tries >= 3 {
                panic!("Test flaked with 'Text file busy' and can be re-run")
            }
            match container_builder.stream_container(&mut file) {
                Err(ContainerBuilderError::CallContainerBuilder(e))
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
