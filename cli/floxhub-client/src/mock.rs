//! Mock server infrastructure for integration testing.
//!
//! This module provides HTTP recording and replay functionality using httpmock.
//! It is only available when the `mock` feature is enabled.

use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;

use httpmock::{MockServer, RecordingID};
use serde_json::Value;
use tracing::debug;

use crate::config::{FloxhubClientConfig, FloxhubMockMode};

/// Guard to keep a `MockServer` running until the `FloxhubClient` is dropped.
#[allow(dead_code)] // https://github.com/rust-lang/rust/issues/122833
pub(crate) enum MockGuard {
    Record(MockRecorder),
    Replay(MockServer),
}

impl MockGuard {
    pub(crate) fn new(config: &FloxhubClientConfig) -> Option<Self> {
        match &config.mock_mode {
            FloxhubMockMode::None => None,
            FloxhubMockMode::Record(path) => {
                let server = MockServer::start();
                server.forward_to(&config.base_url, |rule| {
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
                    base_url: config.base_url.clone(),
                    server,
                    recording,
                    body_redact_keys: Vec::new(),
                };

                Some(MockGuard::Record(recorder))
            },
            FloxhubMockMode::Replay(path) => {
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
            base_url,
            recording,
            ..
        }) = self
        {
            server.reset();
            server.forward_to(base_url.as_str(), |rule| {
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

    /// Configure JSON field names to remove from recorded request bodies.
    ///
    /// When the recorder writes a YAML file, any request whose body is valid
    /// JSON and deep-contains one of the listed keys will have those keys
    /// removed and the matcher switched from exact `body` to
    /// `json_body_includes` (subset matching). Fields absent from the matcher
    /// are not asserted during replay, making the recording stable across
    /// machines even when those fields vary per build.
    ///
    /// Only takes effect in `Record` mode; no-op in `Replay`/`None`.
    pub fn set_body_redact_keys(&mut self, keys: Vec<String>) {
        if let MockGuard::Record(recorder) = self {
            recorder.body_redact_keys = keys;
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
    pub(crate) base_url: String,
    pub(crate) server: MockServer,
    pub(crate) recording: RecordingID,
    /// JSON field names to strip from recorded request bodies.
    ///
    /// See [`MockGuard::set_body_redact_keys`] for the full contract.
    pub(crate) body_redact_keys: Vec<String>,
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

        if self.body_redact_keys.is_empty() {
            fs::rename(&tempfile, &self.path).expect("failed to rename recorded mock file");
        } else {
            let raw = fs::read_to_string(&tempfile).expect("failed to read recorded mock file");
            let redacted = redact_body_keys(&raw, &self.body_redact_keys);
            fs::write(&self.path, redacted).expect("failed to write redacted mock file");
            let _ = fs::remove_file(&tempfile);
        }
        debug!(path = ?self.path, "saved mock recording");
    }
}

/// Post-process a YAML recording file, replacing exact `body` matchers with
/// `json_body_includes` matchers when the body JSON contains any of the given
/// keys (at any nesting depth).
///
/// Each `---`-separated YAML document is processed independently.  Documents
/// whose `when.body` is absent, not valid JSON, or contains none of the
/// volatile keys are left untouched.  For documents that do contain a volatile
/// key the function:
///
/// 1. Parses the body string as JSON.
/// 2. Recursively removes all matching keys from the JSON value.
/// 3. Replaces `when.body` with `when.json_body_includes: [<stripped value>]`.
///
/// The resulting YAML is re-serialized with `serde_yaml`.
pub(crate) fn redact_body_keys(yaml: &str, keys: &[String]) -> String {
    use serde::Deserialize as _;
    use serde_yaml::{Deserializer, Value as YamlValue};

    let mut documents: Vec<String> = Vec::new();

    for doc in Deserializer::from_str(yaml) {
        // Fail loudly on a malformed document rather than silently dropping it
        // from the rewritten recording — this is record-path only and httpmock
        // emits well-formed YAML, so an error here means the recording is
        // corrupt and the mock must not be written. Matches the `.expect()`
        // discipline used by every other IO step in this `Drop`.
        let mut value: YamlValue =
            YamlValue::deserialize(doc).expect("recorded mock document should be valid YAML");

        // Navigate to when.body
        if let YamlValue::Mapping(ref mut root) = value {
            let when_key = YamlValue::String("when".to_string());
            if let Some(YamlValue::Mapping(when)) = root.get_mut(&when_key) {
                let body_key = YamlValue::String("body".to_string());
                if let Some(YamlValue::String(body_str)) = when.get(&body_key).cloned() {
                    // Try to parse as JSON; skip if not JSON or has no volatile keys.
                    if let Ok(mut json_val) = serde_json::from_str::<Value>(&body_str)
                        && json_contains_any_key(&json_val, keys)
                    {
                        remove_keys_recursive(&mut json_val, keys);
                        // Replace body with json_body_includes. Tradeoff:
                        // json_body_includes is an *inclusive* (subset) matcher
                        // — recorded ⊆ actual — so a future regression that
                        // *adds* a field to these narinfo bodies would still
                        // match. Accepted because every content-addressed field
                        // (narHash, narSize, ca, references, …) remains present
                        // and exact-matched; only the four volatile keys above
                        // are dropped.
                        when.remove(&body_key);
                        let includes_key = YamlValue::String("json_body_includes".to_string());
                        // serde_json::Value → serde_yaml::Value via JSON
                        let yaml_json: YamlValue = serde_yaml::to_value(&json_val)
                            .expect("failed to convert JSON to YAML value");
                        when.insert(includes_key, YamlValue::Sequence(vec![yaml_json]));
                    }
                }
            }
        }

        let doc_str = serde_yaml::to_string(&value).expect("failed to re-serialize YAML document");
        documents.push(doc_str);
    }

    documents.join("---\n")
}

/// Returns true if `value` or any nested value is a JSON object containing a
/// key from `keys`.
fn json_contains_any_key(value: &Value, keys: &[String]) -> bool {
    match value {
        Value::Object(map) => {
            keys.iter().any(|k| map.contains_key(k))
                || map.values().any(|v| json_contains_any_key(v, keys))
        },
        Value::Array(arr) => arr.iter().any(|v| json_contains_any_key(v, keys)),
        _ => false,
    }
}

/// Recursively remove all occurrences of `keys` from a JSON value in place.
///
/// Recursion is deliberate: the volatile keys are narinfo ephemera nested under
/// a dynamic store-path key (`narinfos.<path>.registrationTime`, …), so a
/// path-scoped strip would have to hardcode that shape and would silently leave
/// the volatile values in place — breaking replay determinism — if the body
/// shape ever drifted. Stripping wherever the keys appear is the more robust
/// choice for this record-path rewrite. The accepted risk is a same-named field
/// (e.g. `signatures`) legitimately living elsewhere in a future body; revisit
/// the scoping if that ever becomes real.
fn remove_keys_recursive(value: &mut Value, keys: &[String]) {
    match value {
        Value::Object(map) => {
            for key in keys {
                map.remove(key);
            }
            for v in map.values_mut() {
                remove_keys_recursive(v, keys);
            }
        },
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                remove_keys_recursive(v, keys);
            }
        },
        _ => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal two-document recording. The first document has a narinfo body
    /// containing volatile keys; the second has a plain JSON body with none.
    fn sample_yaml() -> String {
        r#"when:
  path: /api/v1/catalog/catalogs/test/packages/pkg/builds
  method: POST
  body: '{"narinfos":{"/nix/store/abc-pkg":{"narHash":"sha256-abc","registrationTime":1783697594,"deriver":"/nix/store/drv-pkg.drv","signatures":[],"ultimate":true}}}'
then:
  status: 201
  body: '{}'
---
when:
  path: /api/v1/catalog/catalogs/test/packages
  method: POST
  body: '{"original_url":"https://example.com/repo.git"}'
then:
  status: 201
  body: '{"name":"pkg"}'
"#
        .to_string()
    }

    #[test]
    fn volatile_body_becomes_json_body_includes() {
        let keys: Vec<String> = vec![
            "registrationTime".to_string(),
            "deriver".to_string(),
            "signatures".to_string(),
            "ultimate".to_string(),
        ];
        let result = redact_body_keys(&sample_yaml(), &keys);

        // The first document should use json_body_includes, not body
        assert!(
            result.contains("json_body_includes"),
            "expected json_body_includes in output:\n{result}"
        );
        // Volatile keys must not appear in the result
        assert!(
            !result.contains("registrationTime"),
            "registrationTime should be stripped:\n{result}"
        );
        assert!(
            !result.contains("deriver"),
            "deriver should be stripped:\n{result}"
        );
        assert!(
            !result.contains("signatures"),
            "signatures should be stripped:\n{result}"
        );
        assert!(
            !result.contains("ultimate"),
            "ultimate should be stripped:\n{result}"
        );
        // Stable fields must survive
        assert!(
            result.contains("narHash"),
            "narHash should be preserved:\n{result}"
        );
    }

    #[test]
    fn stable_body_kept_as_exact_match() {
        let keys: Vec<String> = vec![
            "registrationTime".to_string(),
            "deriver".to_string(),
            "signatures".to_string(),
            "ultimate".to_string(),
        ];
        let result = redact_body_keys(&sample_yaml(), &keys);

        // Parse the output and assert on matcher *structure*, not substring
        // counts: the stable document (no volatile keys) must keep its exact
        // `when.body` matcher, and no document may carry the stable body under
        // a `json_body_includes` (subset) matcher. A substring count of
        // `original_url` holds either way and would not catch that regression.
        use serde::Deserialize as _;
        use serde_yaml::Deserializer;
        let docs: Vec<serde_yaml::Value> = Deserializer::from_str(&result)
            .map(|doc| serde_yaml::Value::deserialize(doc).expect("output must be valid YAML"))
            .collect();

        let stable_kept_as_body = docs.iter().any(|val| {
            val.get("when")
                .and_then(|w| w.get("body"))
                .and_then(|b| b.as_str())
                .is_some_and(|b| b.contains("original_url"))
        });
        assert!(
            stable_kept_as_body,
            "stable document must keep an exact `body` matcher containing \
             original_url:\n{result}"
        );

        let stable_as_includes = docs.iter().any(|val| {
            val.get("when")
                .and_then(|w| w.get("json_body_includes"))
                .map(|inc| {
                    serde_yaml::to_string(inc)
                        .unwrap_or_default()
                        .contains("original_url")
                })
                .unwrap_or(false)
        });
        assert!(
            !stable_as_includes,
            "stable body must NOT be converted to a json_body_includes subset \
             matcher:\n{result}"
        );
    }

    #[test]
    fn round_trip_parses_back_correctly() {
        let keys: Vec<String> = vec![
            "registrationTime".to_string(),
            "deriver".to_string(),
            "signatures".to_string(),
            "ultimate".to_string(),
        ];
        let result = redact_body_keys(&sample_yaml(), &keys);

        // The output must be valid YAML that serde_yaml can round-trip
        use serde::Deserialize as _;
        use serde_yaml::Deserializer;
        let mut doc_count = 0;
        for doc in Deserializer::from_str(&result) {
            let val: serde_yaml::Value =
                serde_yaml::Value::deserialize(doc).expect("output must be valid YAML");
            // Each document must be a mapping with 'when' and 'then'
            let map = val.as_mapping().expect("document must be a mapping");
            assert!(
                map.contains_key(serde_yaml::Value::String("when".to_string())),
                "document must have 'when' key"
            );
            assert!(
                map.contains_key(serde_yaml::Value::String("then".to_string())),
                "document must have 'then' key"
            );
            doc_count += 1;
        }
        assert_eq!(doc_count, 2, "output must contain both documents");
    }

    #[test]
    fn empty_keys_list_leaves_yaml_unchanged() {
        let result = redact_body_keys(&sample_yaml(), &[]);
        // With no keys to redact nothing changes except possibly whitespace;
        // confirm the body strings are still present verbatim.
        assert!(
            result.contains("registrationTime"),
            "registrationTime should be untouched when no keys given:\n{result}"
        );
        assert!(
            result.contains("original_url"),
            "original_url should be untouched when no keys given:\n{result}"
        );
    }

    #[test]
    #[should_panic(expected = "valid YAML")]
    fn malformed_document_panics_instead_of_dropping() {
        // A duplicate mapping key is not a valid YAML document. The recorder
        // must fail loudly rather than silently drop the document from the
        // rewritten recording (which would commit a corrupt mock).
        let keys = vec!["registrationTime".to_string()];
        redact_body_keys("dup: 1\ndup: 2\n", &keys);
    }
}
