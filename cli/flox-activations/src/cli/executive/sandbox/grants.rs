//! Persisted ask-grant storage (`grants.toml`).
//!
//! A grant is a glob pattern the user approved in a prior session. At broker
//! start the file is read once into an in-memory session grant set; a path
//! matching any grant is allowed silently under `ask`, so an environment the
//! user has already trusted stays quiet across activations.
//!
//! This batch is read-only: the broker loads grants but never writes them
//! (writing is a later batch's `flox sandbox` work). The serde types are
//! defined here in full so that later batch extends them rather than
//! redefining the on-disk shape.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// One persisted grant: a glob pattern plus provenance metadata.
///
/// Only `pattern` participates in matching. The remaining fields are recorded
/// for the `flox sandbox list` review surface and for the journal/tamper
/// diff; they are informational and never gate a verdict in this prototype
/// (`FLOX_SANDBOX_ALLOW` globs cannot express read-vs-write, so a saved grant
/// allows all access kinds on its paths — see the design's honest limits).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Grant {
    /// fnmatch-style glob matched against the resolved request path.
    pub pattern: String,

    /// Access kinds the grant was created for (`read`, `write`, `any`).
    /// Informational only in this prototype.
    #[serde(default)]
    pub ops: Vec<String>,

    /// Where the grant came from: review / allow / watch.
    #[serde(default)]
    pub source: Option<String>,

    /// Creation timestamp, free-form (e.g. an ISO date). Informational.
    #[serde(default)]
    pub created: Option<String>,

    /// File count observed at grant time, for the review evidence column.
    #[serde(default)]
    pub evidence: Option<u64>,
}

/// The `grants.toml` document: a versioned list of `[[grant]]` tables.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantsFile {
    /// Schema version, so later batches can migrate the file shape.
    #[serde(default = "default_grants_version")]
    pub version: u32,

    /// The saved grants, one per `[[grant]]` table.
    #[serde(default, rename = "grant")]
    pub grants: Vec<Grant>,
}

fn default_grants_version() -> u32 {
    1
}

impl Default for GrantsFile {
    fn default() -> Self {
        Self {
            version: default_grants_version(),
            grants: Vec::new(),
        }
    }
}

/// The file name the grants live under inside `FLOX_SANDBOX_GRANTS_DIR`.
pub const GRANTS_FILE_NAME: &str = "grants.toml";

/// Read `grants.toml` from `grants_dir` into a [`GrantsFile`].
///
/// A missing file is normal (no grants yet) and yields an empty set rather
/// than an error. A present-but-malformed file is logged and treated as
/// empty: a broker that fails to parse its grants must still serve verdicts
/// (fail toward asking, never toward silently allowing on a parse bug).
pub fn read_grants(grants_dir: &Path) -> GrantsFile {
    let path = grants_dir.join(GRANTS_FILE_NAME);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            debug!(?path, "no grants.toml; starting with an empty grant set");
            return GrantsFile::default();
        },
        Err(err) => {
            warn!(?path, %err, "could not read grants.toml; treating as empty");
            return GrantsFile::default();
        },
    };
    match toml::from_str::<GrantsFile>(&contents) {
        Ok(file) => file,
        Err(err) => {
            warn!(?path, %err, "could not parse grants.toml; treating as empty");
            GrantsFile::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_grants_file_is_an_empty_set() {
        let tmp = tempfile::tempdir().unwrap();
        let grants = read_grants(tmp.path());
        assert_eq!(grants, GrantsFile::default());
        assert!(grants.grants.is_empty());
    }

    #[test]
    fn reads_grants_with_full_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(GRANTS_FILE_NAME),
            r#"
version = 1

[[grant]]
pattern = "~/.cargo/registry/**"
ops = ["read"]
source = "flox sandbox review"
created = "2026-06-11"
evidence = 214

[[grant]]
pattern = "~/data/fixtures/**"
ops = ["any"]
source = "flox sandbox allow"
"#,
        )
        .unwrap();

        let grants = read_grants(tmp.path());
        assert_eq!(grants.version, 1);
        assert_eq!(grants.grants, vec![
            Grant {
                pattern: "~/.cargo/registry/**".to_string(),
                ops: vec!["read".to_string()],
                source: Some("flox sandbox review".to_string()),
                created: Some("2026-06-11".to_string()),
                evidence: Some(214),
            },
            Grant {
                pattern: "~/data/fixtures/**".to_string(),
                ops: vec!["any".to_string()],
                source: Some("flox sandbox allow".to_string()),
                created: None,
                evidence: None,
            },
        ]);
    }

    #[test]
    fn minimal_grant_needs_only_a_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(GRANTS_FILE_NAME),
            "[[grant]]\npattern = \"/data/**\"\n",
        )
        .unwrap();

        let grants = read_grants(tmp.path());
        assert_eq!(grants.grants.len(), 1);
        assert_eq!(grants.grants[0].pattern, "/data/**");
        assert!(grants.grants[0].ops.is_empty());
    }

    #[test]
    fn malformed_grants_file_is_treated_as_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(GRANTS_FILE_NAME),
            "this is not valid toml = = =\n",
        )
        .unwrap();

        // A broker must keep serving verdicts even if the grants file is
        // corrupt; it falls back to an empty set (everything goes through ask)
        // rather than erroring or silently allowing.
        let grants = read_grants(tmp.path());
        assert!(grants.grants.is_empty());
    }
}
