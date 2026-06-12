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

/// The `kind` value marking a grant as a network destination rather than a
/// filesystem glob. A missing/`fs` kind is a filesystem grant.
pub const KIND_NET: &str = "net";

/// The `source` value stamped on grants written by the one-time default
/// seeding, so the review surface can tell out-of-box policy from grants the
/// user approved.
pub const SOURCE_DEFAULT_SEED: &str = "default-seed";

/// One persisted grant: a glob pattern plus provenance metadata.
///
/// Only `pattern` participates in matching. The remaining fields are recorded
/// for the `flox sandbox list` review surface and for the journal/tamper
/// diff; they are informational and never gate a verdict in this prototype
/// (`FLOX_SANDBOX_ALLOW` globs cannot express read-vs-write, so a saved grant
/// allows all access kinds on its paths — see the design's honest limits).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Grant {
    /// fnmatch-style glob matched against the resolved request path, or (for
    /// `kind = "net"`) a network destination (`host[:port]`).
    pub pattern: String,

    /// What the pattern names: `None`/`"fs"` for a filesystem glob, `"net"`
    /// for a network destination. Network grants are compiled into
    /// `FLOX_SANDBOX_ALLOW_NET` and never count against the filesystem
    /// allow-set caps.
    #[serde(default)]
    pub kind: Option<String>,

    /// Access kinds the grant was created for (`read`, `write`, `any`).
    /// Informational only in this prototype.
    #[serde(default)]
    pub ops: Vec<String>,

    /// Where the grant came from: review / allow / watch / default-seed.
    #[serde(default)]
    pub source: Option<String>,

    /// Creation timestamp, free-form (e.g. an ISO date). Informational.
    #[serde(default)]
    pub created: Option<String>,

    /// File count observed at grant time, for the review evidence column.
    #[serde(default)]
    pub evidence: Option<u64>,
}

impl Grant {
    /// True when this grant names a network destination (`kind = "net"`)
    /// rather than a filesystem glob.
    pub fn is_net(&self) -> bool {
        self.kind.as_deref() == Some(KIND_NET)
    }

    /// True when this grant was written by the default seeding rather than
    /// approved by the user.
    pub fn is_default_seed(&self) -> bool {
        self.source.as_deref() == Some(SOURCE_DEFAULT_SEED)
    }
}

/// The `grants.toml` document: a versioned list of `[[grant]]` tables.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantsFile {
    /// Schema version, so later batches can migrate the file shape.
    #[serde(default = "default_grants_version")]
    pub version: u32,

    /// The default-seed generation already applied to this file. Re-seeding
    /// is gated on this marker — never on entry presence — so a user's
    /// revocation of a seeded grant survives later activations (a
    /// presence-based reseed would silently resurrect it).
    #[serde(default)]
    pub seeded_version: u32,

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
            seeded_version: 0,
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

/// The default-seed generation this build writes. Bump when the default
/// grant set changes shape; environments seeded at an older generation will
/// be topped up with the new entries (existing user edits untouched).
pub const SEED_GRANTS_VERSION: u32 = 1;

/// Network destinations seeded as explicit, revocable `[[grant]]` entries on
/// the first sandboxed activation of an environment.
///
/// These are *policy* — convenience allowances a user may legitimately want
/// to revoke — as opposed to the infrastructure entries (loopback and flox's
/// own service hosts) that stay hardcoded in the seed because revoking them
/// would break flox itself. Two groups:
///
/// - Git hosting and release downloads: clone/fetch/pull and release
///   archives (GitHub plus its CDN hosts).
/// - Language package registries: npm, PyPI (index + file CDN), and
///   crates.io (index + downloads), so an agent can install dependencies
///   without a manual grant.
const NET_SEED_GRANTS: &[&str] = &[
    "github.com",
    "codeload.github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
    "registry.npmjs.org",
    "pypi.org",
    "files.pythonhosted.org",
    "crates.io",
    "static.crates.io",
    "index.crates.io",
];

/// Shell rc, profile, and history files seeded as `$HOME`-expanded grants.
///
/// Under `ask` the engine flips the `$HOME`-dotfile carve-out, so without
/// these the first interactive shell would queue a receipt for reading its
/// own startup files. Covers the zsh and bash families plus the shared
/// `.profile`/`.inputrc`.
const SHELL_DOTFILE_SEEDS: &[&str] = &[
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zsh_history",
    ".bashrc",
    ".bash_profile",
    ".bash_history",
    ".profile",
    ".inputrc",
];

/// Routine, non-sensitive developer config files seeded as `$HOME`-expanded
/// grants.
///
/// These are deliberately *non-sensitive*: they hold tool preferences
/// (editor, registry URL, build profile), never credentials. Secrets live in
/// the sensitive set the engine denies even under `enforce` (`~/.ssh`,
/// `~/.aws`, `~/.netrc`, `~/.config/gh`, `**/.env`, ...), which is why none
/// of those appears here — seeding a credential path would defeat the
/// denial.
const DEV_CONFIG_SEEDS: &[&str] = &[
    // git: user config and the XDG config dir (excludes ~/.config/gh, which
    // is sensitive and seeded nowhere).
    ".gitconfig",
    ".config/git/**",
    // npm.
    ".npmrc",
    ".config/npm/**",
    // cargo: both the legacy and current config filenames.
    ".cargo/config",
    ".cargo/config.toml",
    // pip.
    ".config/pip/**",
    ".pip/**",
    // rustup toolchain selection.
    ".rustup/settings.toml",
];

/// The default grant set written by the one-time seeding: the implicit
/// policy (git hosts, registries, dotfile reads, and flox's own metrics
/// endpoint) made explicit, inspectable, and revocable.
///
/// Filesystem patterns are written `$HOME`-expanded and absolute because the
/// broker matches grants literally against realpaths, with no `~` expansion.
/// `metrics_host` is the hostname of flox's metrics endpoint, passed in by
/// the CLI (`None` when the user disabled metrics): without it, every
/// short-lived flox process inside an `enforce` session bursts connection
/// refusals when the telemetry buffer tries to flush.
pub fn default_seed_grants(home: Option<&Path>, metrics_host: Option<&str>) -> Vec<Grant> {
    let today = today();
    let net_grant = |host: &str| Grant {
        pattern: host.to_string(),
        kind: Some(KIND_NET.to_string()),
        ops: Vec::new(),
        source: Some(SOURCE_DEFAULT_SEED.to_string()),
        created: Some(today.clone()),
        evidence: None,
    };
    let fs_grant = |pattern: String| Grant {
        pattern,
        kind: None,
        ops: vec!["read".to_string()],
        source: Some(SOURCE_DEFAULT_SEED.to_string()),
        created: Some(today.clone()),
        evidence: None,
    };

    let mut grants: Vec<Grant> = NET_SEED_GRANTS.iter().map(|host| net_grant(host)).collect();
    // The metrics host is flox's own service traffic, but it is seeded as a
    // visible grant like the rest of the default policy — not a hardcoded
    // exemption — so it shows up in `flox sandbox list --all` and can be
    // revoked.
    if let Some(host) = metrics_host {
        grants.push(net_grant(host));
    }

    if let Some(home) = home {
        for dotfile in SHELL_DOTFILE_SEEDS {
            if let Some(pattern) = home.join(dotfile).to_str() {
                grants.push(fs_grant(pattern.to_string()));
            }
        }
        for config in DEV_CONFIG_SEEDS {
            if let Some(pattern) = home.join(config).to_str() {
                grants.push(fs_grant(pattern.to_string()));
            }
        }
    }
    grants
}

/// Write the default seed grants into `grants_dir/grants.toml`, once per
/// seed generation.
///
/// Idempotence and revocability hang on the `seeded_version` gate: when the
/// file already records this generation, nothing happens — even if seeded
/// entries were since deleted. Revoking a seeded grant is therefore just the
/// ordinary revoke path, and it stays revoked. When seeding does run, only
/// patterns not already present are appended (a user's earlier manual grant
/// for the same pattern is left untouched), and each appended grant is
/// journaled so the tamper diff does not flag the seeds as hand-edits.
///
/// Returns `true` when the file was (re)written.
pub fn ensure_seed_grants(
    grants_dir: &Path,
    home: Option<&Path>,
    metrics_host: Option<&str>,
) -> anyhow::Result<bool> {
    let mut file = read_grants(grants_dir);
    if file.seeded_version >= SEED_GRANTS_VERSION {
        return Ok(false);
    }

    let mut appended: Vec<Grant> = Vec::new();
    for seed in default_seed_grants(home, metrics_host) {
        if file
            .grants
            .iter()
            .any(|grant| grant.pattern == seed.pattern)
        {
            continue;
        }
        appended.push(seed.clone());
        file.grants.push(seed);
    }
    file.seeded_version = SEED_GRANTS_VERSION;
    write_grants(grants_dir, &file)?;
    for grant in &appended {
        append_journal(grants_dir, &JournalRecord {
            event: "grant".to_string(),
            pattern: Some(grant.pattern.clone()),
            source: Some(SOURCE_DEFAULT_SEED.to_string()),
            created: grant.created.clone(),
        });
    }
    debug!(
        ?grants_dir,
        seeded = appended.len(),
        "seeded default sandbox grants"
    );
    Ok(true)
}

/// Today's date as `YYYY-MM-DD`, for the `created` stamp on seeded grants.
fn today() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month() as u8,
        now.day()
    )
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

/// The audit store beside `grants.toml`: NDJSON records appended by the
/// engine (libsandbox) for every warn-mode report and enforce/ask denial.
pub const AUDIT_FILE_NAME: &str = "audit.ndjson";

/// One audit record as written by the engine.
///
/// The engine builds these lines by hand in C, so the reader is tolerant:
/// every field except `path` defaults, and a malformed line is skipped on
/// read rather than failing the whole audit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditRecord {
    /// Unix seconds when the report was emitted.
    #[serde(default)]
    pub ts: i64,
    /// The sandbox mode at the time (`warn` / `enforce` / `ask`).
    #[serde(default)]
    pub mode: String,
    /// `fs` or `net`.
    #[serde(default)]
    pub kind: String,
    /// `read` / `write` / `connect`.
    #[serde(default)]
    pub op: String,
    /// The resolved path (fs) or `host:port` destination (net).
    pub path: String,
    /// `warned` / `denied` / `fail-closed`.
    #[serde(default)]
    pub verdict: String,
    /// The reporting process id.
    #[serde(default)]
    pub pid: i64,
    /// The reporting executable's realpath, or empty.
    #[serde(default)]
    pub exe: String,
}

/// Read every parseable record from `grants_dir/audit.ndjson`, in file
/// order.
///
/// A missing file is an empty audit (nothing was denied). Malformed or
/// truncated lines — a crash mid-append, or a record from a newer engine —
/// are skipped rather than failing the read, matching how the journal is
/// consumed.
pub fn read_audit(grants_dir: &Path) -> Vec<AuditRecord> {
    let path = grants_dir.join(AUDIT_FILE_NAME);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(_) => return Vec::new(),
    };
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<AuditRecord>(line).ok())
        .collect()
}

/// Remove `grants_dir/audit.ndjson`, leaving `grants.toml` and the journal
/// untouched. Clearing the audit never touches grants: the audit is a log of
/// past reports, not policy. A missing file is a no-op success.
pub fn clear_audit(grants_dir: &Path) -> anyhow::Result<()> {
    let path = grants_dir.join(AUDIT_FILE_NAME);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
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
                kind: None,
                ops: vec!["read".to_string()],
                source: Some("flox sandbox review".to_string()),
                created: Some("2026-06-11".to_string()),
                evidence: Some(214),
            },
            Grant {
                pattern: "~/data/fixtures/**".to_string(),
                kind: None,
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
            seeded_version: 0,
            grants: vec![
                Grant {
                    pattern: "/home/dev/.cargo/registry/**".to_string(),
                    kind: None,
                    ops: vec!["read".to_string()],
                    source: Some("review".to_string()),
                    created: Some("2026-06-11".to_string()),
                    evidence: Some(214),
                },
                Grant {
                    pattern: "/home/dev/data/**".to_string(),
                    kind: Some(KIND_NET.to_string()),
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
            kind: None,
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
            kind: None,
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
            seeded_version: 0,
            grants: vec![Grant {
                pattern: "/home/dev/**".to_string(),
                kind: None,
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

    #[test]
    fn seeding_writes_default_grants_with_kind_source_and_journal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache").join("sandbox");
        let home = Path::new("/home/dev");

        let seeded = ensure_seed_grants(&dir, Some(home), Some("metrics.example.com")).unwrap();
        assert!(seeded);

        let file = read_grants(&dir);
        assert_eq!(file.seeded_version, SEED_GRANTS_VERSION);

        // Every default-seed entry carries the provenance source and the
        // right kind: net for hosts, fs (None) for $HOME-expanded patterns.
        assert!(!file.grants.is_empty());
        for grant in &file.grants {
            assert_eq!(
                grant.source.as_deref(),
                Some(SOURCE_DEFAULT_SEED),
                "unexpected source on {}",
                grant.pattern
            );
        }
        let net: Vec<&str> = file
            .grants
            .iter()
            .filter(|g| g.is_net())
            .map(|g| g.pattern.as_str())
            .collect();
        for host in ["github.com", "registry.npmjs.org", "crates.io"] {
            assert!(net.contains(&host), "missing net seed {host}: {net:?}");
        }
        // The metrics host is an ordinary, visible net grant — not a
        // hardcoded exemption.
        assert!(net.contains(&"metrics.example.com"), "got {net:?}");

        // Filesystem seeds are $HOME-expanded and absolute (the broker
        // matches literally against realpaths).
        let fs: Vec<&str> = file
            .grants
            .iter()
            .filter(|g| !g.is_net())
            .map(|g| g.pattern.as_str())
            .collect();
        assert!(fs.contains(&"/home/dev/.zshrc"), "got {fs:?}");
        assert!(fs.contains(&"/home/dev/.gitconfig"), "got {fs:?}");
        assert!(fs.contains(&"/home/dev/.config/git/**"), "got {fs:?}");

        // Each seeded grant is journaled, so the tamper diff has nothing to
        // flag.
        assert!(unjournaled_patterns(&dir).is_empty());
    }

    #[test]
    fn seeding_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache").join("sandbox");
        let home = Path::new("/home/dev");

        assert!(ensure_seed_grants(&dir, Some(home), None).unwrap());
        let first = read_grants(&dir);
        // A second call at the same generation is a no-op.
        assert!(!ensure_seed_grants(&dir, Some(home), None).unwrap());
        assert_eq!(read_grants(&dir), first);
    }

    #[test]
    fn a_revoked_seed_grant_stays_revoked_across_reseeding() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache").join("sandbox");
        let home = Path::new("/home/dev");

        ensure_seed_grants(&dir, Some(home), None).unwrap();

        // Revoke github.com (the ordinary revoke path: drop the entry).
        let mut file = read_grants(&dir);
        file.grants.retain(|grant| grant.pattern != "github.com");
        write_grants(&dir, &file).unwrap();

        // Re-running the seeding must NOT resurrect it: the gate is the
        // seeded_version marker, not entry presence.
        assert!(!ensure_seed_grants(&dir, Some(home), None).unwrap());
        let after = read_grants(&dir);
        assert!(
            after.grants.iter().all(|g| g.pattern != "github.com"),
            "revoked seed grant was resurrected: {:?}",
            after.grants
        );
    }

    #[test]
    fn seeding_skips_patterns_the_user_already_granted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache").join("sandbox");
        let home = Path::new("/home/dev");

        // A manual grant for a pattern the seed would also write.
        let mut file = GrantsFile::default();
        file.grants.push(Grant {
            pattern: "github.com".to_string(),
            kind: Some(KIND_NET.to_string()),
            ops: vec![],
            source: Some("allow".to_string()),
            created: None,
            evidence: None,
        });
        write_grants(&dir, &file).unwrap();

        ensure_seed_grants(&dir, Some(home), None).unwrap();
        let after = read_grants(&dir);
        let github: Vec<&Grant> = after
            .grants
            .iter()
            .filter(|g| g.pattern == "github.com")
            .collect();
        // Not duplicated, and the user's provenance is preserved.
        assert_eq!(github.len(), 1);
        assert_eq!(github[0].source.as_deref(), Some("allow"));
    }

    #[test]
    fn no_sensitive_path_is_ever_seeded() {
        let home = Path::new("/home/dev");
        let grants = default_seed_grants(Some(home), Some("metrics.example.com"));
        // Seeding a credential path would defeat the engine's sensitive-set
        // denial, so none may appear in the default grants.
        let forbidden_fragments = [
            "/.ssh",
            "/.aws",
            "/.gnupg",
            "/.kube",
            "/.netrc",
            "/.config/gh",
            ".env",
        ];
        for fragment in forbidden_fragments {
            assert!(
                grants.iter().all(|g| !g.pattern.contains(fragment)),
                "sensitive fragment {fragment:?} leaked into the seed grants"
            );
        }
    }

    #[test]
    fn audit_read_skips_malformed_lines_and_clear_leaves_grants_intact() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cache").join("sandbox");
        std::fs::create_dir_all(&dir).unwrap();

        // Grants and journal exist alongside the audit.
        ensure_seed_grants(&dir, Some(Path::new("/home/dev")), None).unwrap();
        let grants_before = read_grants(&dir);

        // Two clean engine-shaped records, one malformed line, one truncated
        // final line (crash mid-append).
        std::fs::write(
            dir.join(AUDIT_FILE_NAME),
            concat!(
                "{\"ts\":1781240000,\"mode\":\"enforce\",\"kind\":\"fs\",",
                "\"op\":\"read\",\"path\":\"/home/dev/secret\",",
                "\"verdict\":\"denied\",\"pid\":42,\"exe\":\"/bin/cat\"}\n",
                "not json at all\n",
                "{\"ts\":1781240001,\"mode\":\"warn\",\"kind\":\"net\",",
                "\"op\":\"connect\",\"path\":\"example.com:443\",",
                "\"verdict\":\"warned\",\"pid\":43,\"exe\":\"/bin/curl\"}\n",
                "{\"ts\":1781240002,\"mo",
            ),
        )
        .unwrap();

        let records = read_audit(&dir);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0], AuditRecord {
            ts: 1781240000,
            mode: "enforce".to_string(),
            kind: "fs".to_string(),
            op: "read".to_string(),
            path: "/home/dev/secret".to_string(),
            verdict: "denied".to_string(),
            pid: 42,
            exe: "/bin/cat".to_string(),
        });
        assert_eq!(records[1].kind, "net");
        assert_eq!(records[1].path, "example.com:443");

        // Clearing the audit removes only audit.ndjson: grants.toml and the
        // journal are untouched (--clear never revokes anything).
        clear_audit(&dir).unwrap();
        assert!(read_audit(&dir).is_empty());
        assert!(!dir.join(AUDIT_FILE_NAME).exists());
        assert_eq!(read_grants(&dir), grants_before);
        assert!(dir.join(JOURNAL_FILE_NAME).exists());

        // Clearing an already-missing audit is a no-op success.
        clear_audit(&dir).unwrap();
    }

    #[test]
    fn pre_seeding_grants_file_reads_with_seeded_version_zero() {
        let tmp = tempfile::tempdir().unwrap();
        // A grants.toml written before seeding existed has no seeded_version
        // key; it must read as generation 0 (eligible for seeding).
        std::fs::write(
            tmp.path().join(GRANTS_FILE_NAME),
            "version = 1\n\n[[grant]]\npattern = \"/data/**\"\n",
        )
        .unwrap();
        let file = read_grants(tmp.path());
        assert_eq!(file.seeded_version, 0);
        assert_eq!(file.grants.len(), 1);
    }
}
