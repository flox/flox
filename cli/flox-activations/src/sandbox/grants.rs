//! Persisted ask-grant storage (`grants.toml`) and its provenance journal.
//!
//! A grant is a glob pattern the user approved in a prior session. At broker
//! start the file is read once into an in-memory session grant set; a path
//! matching any grant is allowed silently under `ask`, so an environment the
//! user has already trusted stays quiet across activations.
//!
//! Two files live side by side under `FLOX_SANDBOX_GRANTS_DIR`:
//!
//! - `grants.toml` — the authoritative, hand-editable grant set. Written
//!   atomically (temp + rename) so a concurrent reader never sees a partial
//!   file.
//! - `journal.ndjson` — an append-only record of every grant and verdict,
//!   keyed by pattern. It powers tamper-evidence: a grant present in
//!   `grants.toml` but absent from the journal was added outside flox (a
//!   hand-edit, or a self-approving agent), and the activation banner surfaces
//!   it. The journal is provenance, not policy: it never gates a verdict.

use std::io::Write;
use std::path::{Path, PathBuf};

use flox_core::write_atomically;
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

/// The append-only provenance journal beside `grants.toml`.
pub const JOURNAL_FILE_NAME: &str = "journal.ndjson";

/// One journal record: a grant or verdict, appended verbatim.
///
/// Records are newline-delimited JSON so the file is greppable and a partial
/// final line (from a crash mid-append) is simply ignored on read. Only
/// `pattern` participates in the tamper diff; the rest is provenance an
/// operator can inspect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalRecord {
    /// `grant` (a pattern was saved) or `verdict` (a decision was made).
    pub event: String,
    /// The grant pattern this record concerns, when it is a grant.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Where the grant came from: review / allow / watch.
    #[serde(default)]
    pub source: Option<String>,
    /// Free-form timestamp, stamped by the caller.
    #[serde(default)]
    pub created: Option<String>,
}

/// Write `file` to `grants_dir/grants.toml` atomically (temp + rename).
///
/// The directory is created if missing. A write replaces the whole file: the
/// caller mutates the in-memory [`GrantsFile`] and writes it back, rather than
/// editing in place, so a reader always sees a consistent document. The rename
/// is atomic on the same filesystem, which `grants_dir` always is (it lives
/// under the environment's `.flox/cache`).
pub fn write_grants(grants_dir: &Path, file: &GrantsFile) -> anyhow::Result<()> {
    std::fs::create_dir_all(grants_dir)?;
    let path = grants_dir.join(GRANTS_FILE_NAME);
    let body = toml::to_string_pretty(file)?;
    write_atomically(&path, body)?;
    debug!(?path, count = file.grants.len(), "wrote grants.toml");
    Ok(())
}

/// Append one record to `grants_dir/journal.ndjson`, creating it if missing.
///
/// Best-effort and append-only: a failure to journal is logged but never
/// fails the grant write, because losing provenance must not block an
/// approval. A missing journal entry for a present grant is exactly the
/// tamper signal the activation banner reports, so a dropped append degrades
/// to a false-positive warning, never a silent allow.
pub fn append_journal(grants_dir: &Path, record: &JournalRecord) {
    if let Err(err) = append_journal_inner(grants_dir, record) {
        warn!(%err, "could not append to sandbox journal; provenance may be incomplete");
    }
}

fn append_journal_inner(grants_dir: &Path, record: &JournalRecord) -> anyhow::Result<()> {
    std::fs::create_dir_all(grants_dir)?;
    let path = grants_dir.join(JOURNAL_FILE_NAME);
    let mut line = serde_json::to_string(record)?;
    line.push('\n');
    let mut handle = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    handle.write_all(line.as_bytes())?;
    Ok(())
}

/// Read every grant pattern recorded in the journal.
///
/// A malformed or partial final line is skipped rather than failing the whole
/// read — the journal is append-only, so a crash can only ever truncate the
/// last record. A missing journal yields an empty set (every grant then looks
/// unjournaled, which is the correct conservative signal for a file that has
/// grants but no provenance at all).
pub fn journaled_patterns(grants_dir: &Path) -> std::collections::HashSet<String> {
    let path = grants_dir.join(JOURNAL_FILE_NAME);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(_) => return std::collections::HashSet::new(),
    };
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<JournalRecord>(line).ok())
        .filter(|record| record.event == "grant")
        .filter_map(|record| record.pattern)
        .collect()
}

/// Patterns present in `grants.toml` but absent from the journal.
///
/// These were added outside flox — a hand-edit, or a self-approving agent that
/// wrote `grants.toml` directly without going through the broker (which always
/// journals). The activation banner surfaces them as "possibly self-approved".
/// Order follows the grants file so the warning is stable.
pub fn unjournaled_patterns(grants_dir: &Path) -> Vec<String> {
    let journaled = journaled_patterns(grants_dir);
    read_grants(grants_dir)
        .grants
        .into_iter()
        .map(|grant| grant.pattern)
        .filter(|pattern| !journaled.contains(pattern))
        .collect()
}

/// The path to `grants.toml` inside `grants_dir`, for callers that need to
/// report it (e.g. the `flox sandbox list` header).
pub fn grants_file_path(grants_dir: &Path) -> PathBuf {
    grants_dir.join(GRANTS_FILE_NAME)
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

    #[test]
    fn write_then_read_round_trips_grants() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache").join("sandbox");
        let file = GrantsFile {
            version: 1,
            grants: vec![
                Grant {
                    pattern: "/home/dev/.cargo/registry/**".to_string(),
                    ops: vec!["read".to_string()],
                    source: Some("review".to_string()),
                    created: Some("2026-06-11".to_string()),
                    evidence: Some(214),
                },
                Grant {
                    pattern: "/home/dev/data/**".to_string(),
                    ops: vec!["any".to_string()],
                    source: Some("allow".to_string()),
                    created: None,
                    evidence: None,
                },
            ],
        };

        // The writer creates the directory, so the round-trip works even when
        // the cache dir does not exist yet (the first grant in a fresh env).
        write_grants(&dir, &file).unwrap();
        let read_back = read_grants(&dir);
        assert_eq!(read_back, file);
    }

    #[test]
    fn journaled_grant_is_not_flagged_but_hand_edited_grant_is() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache").join("sandbox");

        // A grant written through the broker is journaled, so it is trusted.
        let mut file = GrantsFile::default();
        file.grants.push(Grant {
            pattern: "/home/dev/project/**".to_string(),
            ops: vec!["read".to_string()],
            source: Some("allow".to_string()),
            created: Some("2026-06-11".to_string()),
            evidence: None,
        });
        write_grants(&dir, &file).unwrap();
        append_journal(&dir, &JournalRecord {
            event: "grant".to_string(),
            pattern: Some("/home/dev/project/**".to_string()),
            source: Some("allow".to_string()),
            created: Some("2026-06-11".to_string()),
        });

        // A grant hand-edited into the file never reaches the journal.
        file.grants.push(Grant {
            pattern: "/home/dev/secrets/**".to_string(),
            ops: vec!["any".to_string()],
            source: None,
            created: None,
            evidence: None,
        });
        write_grants(&dir, &file).unwrap();

        let unjournaled = unjournaled_patterns(&dir);
        assert_eq!(unjournaled, vec!["/home/dev/secrets/**".to_string()]);
    }

    #[test]
    fn a_grants_file_with_no_journal_flags_everything() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache").join("sandbox");
        let file = GrantsFile {
            version: 1,
            grants: vec![Grant {
                pattern: "/home/dev/**".to_string(),
                ops: vec![],
                source: None,
                created: None,
                evidence: None,
            }],
        };
        write_grants(&dir, &file).unwrap();

        // No journal at all: every grant is unexplained provenance, which is
        // the correct conservative signal (the banner will list them all).
        assert_eq!(unjournaled_patterns(&dir), vec!["/home/dev/**".to_string()]);
    }

    #[test]
    fn journal_skips_a_truncated_final_record() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache").join("sandbox");
        std::fs::create_dir_all(&dir).unwrap();
        // A clean record followed by a truncated one (crash mid-append).
        std::fs::write(
            dir.join(JOURNAL_FILE_NAME),
            "{\"event\":\"grant\",\"pattern\":\"/a/**\"}\n{\"event\":\"gr",
        )
        .unwrap();

        let journaled = journaled_patterns(&dir);
        assert!(journaled.contains("/a/**"));
        assert_eq!(journaled.len(), 1);
    }
}
