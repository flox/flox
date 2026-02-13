//! Mock server infrastructure for integration testing.
//!
//! This module provides HTTP recording and replay functionality using httpmock.
//! It is only available when the `mock` feature is enabled.

use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;

use httpmock::{MockServer, RecordingID};
use tracing::debug;

use crate::config::{CatalogClientConfig, CatalogMockMode};

/// Guard to keep a `MockServer` running until the `CatalogClient` is dropped.
#[allow(dead_code)] // https://github.com/rust-lang/rust/issues/122833
pub(crate) enum MockGuard {
    Record(MockRecorder),
    Replay(MockServer),
}

impl MockGuard {
    pub(crate) fn new(config: &CatalogClientConfig) -> Option<Self> {
        match &config.mock_mode {
            CatalogMockMode::None => None,
            CatalogMockMode::Record(path) => {
                let server = MockServer::start();
                server.forward_to(&config.catalog_url, |rule| {
                    rule.filter(|when| {
                        when.any_request();
                    });
                });
                let recording = server.record(|rule| {
                    rule.filter(|when| {
                        when.any_request();
                    });
                });

                debug!(?path, server = server.base_url(), "mock server recording");
                let recorder = MockRecorder {
                    path: path.to_path_buf(),
                    catalog_url: config.catalog_url.clone(),
                    server,
                    recording,
                };

                Some(MockGuard::Record(recorder))
            },
            CatalogMockMode::Replay(path) => {
                let server = MockServer::start();
                server.playback(path);
                debug!(?path, server = server.base_url(), "mock server replaying");

                Some(MockGuard::Replay(server))
            },
        }
    }

    pub(crate) fn url(&self) -> String {
        match self {
            MockGuard::Record(recorder) => recorder.server.base_url().to_string(),
            MockGuard::Replay(server) => server.base_url().to_string(),
        }
    }

    /// Clear everything that has been recorded up to this point.
    ///
    /// This is useful in tests where you need to perform some setup that
    /// you don't want to be included as part of the recording e.g. test
    /// initialization.
    pub fn reset_recording(&mut self) {
        if let MockGuard::Record(MockRecorder {
            server,
            catalog_url,
            recording,
            ..
        }) = self
        {
            server.reset();
            server.forward_to(catalog_url.as_str(), |rule| {
                rule.filter(|when| {
                    when.any_request();
                });
            });
            let new_recording = server.record(|rule| {
                rule.filter(|when| {
                    when.any_request();
                });
            });
            *recording = new_recording;
        }
    }
}

impl Debug for MockGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let url = self.url();
        let mode = match self {
            MockGuard::Record(_) => "MockGuard::Record",
            MockGuard::Replay(_) => "MockGuard::Replay",
        };
        write!(f, "{mode} url={url}")
    }
}

/// In addition to keeping a `MockServer` running, also write any recorded
/// requests to a file when dropped.
pub(crate) struct MockRecorder {
    pub(crate) path: PathBuf,
    pub(crate) catalog_url: String,
    pub(crate) server: MockServer,
    pub(crate) recording: RecordingID,
}

impl Drop for MockRecorder {
    fn drop(&mut self) {
        // `save` and `save_to` append a timestamp, so we rename after write.
        // https://github.com/alexliesenfeld/httpmock/issues/115
        let tempfile = self
            .server
            .record_save(
                &self.recording,
                // We need something unique in the name otherwise parallel
                // threads can race each other
                format!(
                    "httpmock_{}",
                    self.path
                        .file_name()
                        .expect("path should have filename")
                        .to_str()
                        .expect("path should be unicode")
                ),
            )
            .expect("failed to save mock recording");
        debug!(
            src = %tempfile.as_path().display(),
            dest = %self.path.as_path().display(),
            src_exists = tempfile.as_path().exists(),
            "renaming recorded mock file"
        );
        fs::rename(&tempfile, &self.path).expect("failed to rename recorded mock file");
        debug!(path = ?self.path, "saved mock recording");
    }
}
