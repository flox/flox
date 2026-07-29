//! The local environment's stable id, stored as a bare UUID string in
//! `.flox/telemetry_id`.
//!
//! Minted in the CLI layer when a path environment is created (`flox init`,
//! `flox pull --copy`) and read at event-emit time to populate
//! `local_environment_id`. It lives in the committed `.flox` alongside
//! `env.json`, so it travels with a git-clone or folder-copy, but it is
//! deliberately not part of the environment pointer, and the SDK is not
//! involved in minting or reading it.

use flox_core::data::CanonicalPath;
use flox_core::write_atomically;
use tracing::debug;
use uuid::Uuid;

/// File inside `.flox` holding the environment's stable local id.
pub(crate) const TELEMETRY_ID_FILENAME: &str = "telemetry_id";

/// Read the local environment id from `<dot_flox>/telemetry_id`. Best-effort
/// and read-only: a missing or malformed file yields `None`, never an error,
/// and reading never writes.
pub(crate) fn read(dot_flox: &CanonicalPath) -> Option<Uuid> {
    let contents = std::fs::read_to_string(dot_flox.join(TELEMETRY_ID_FILENAME)).ok()?;
    Uuid::try_parse(contents.trim()).ok()
}

/// Whether an environment definition starts at this `.flox` or was already
/// identified. `flox pull --copy` can convert in place, so a directory that
/// was `flox init`-ed and then pushed keeps its original id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Definition {
    /// No id was present, so this call minted one. A write failure still
    /// reports `New`: the definition is new, it simply carries no id.
    New,
    Existing,
}

/// Ensure a newly created path environment has a stable local id at
/// `<dot_flox>/telemetry_id`. Idempotent: an already-identified environment
/// keeps its existing id. Best-effort: a write failure is logged, and that
/// environment then carries no id for its lifetime (reads never write, so
/// nothing recreates it).
pub(crate) fn ensure(dot_flox: &CanonicalPath) -> Definition {
    if read(dot_flox).is_some() {
        return Definition::Existing;
    }
    let id = Uuid::new_v4();
    if let Err(err) = write_atomically(dot_flox.join(TELEMETRY_ID_FILENAME), format!("{id}\n")) {
        debug!(error = %err, "could not write local_environment_id");
    }
    Definition::New
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn ensure_creates_readable_id() {
        let dir = tempdir().unwrap();
        let dot_flox = CanonicalPath::new(dir.path()).unwrap();
        assert_eq!(read(&dot_flox), None, "no id before minting");
        assert_eq!(ensure(&dot_flox), Definition::New);
        assert_ne!(read(&dot_flox), None, "id exists after ensuring");
    }

    #[test]
    fn ensure_keeps_existing_id() {
        let dir = tempdir().unwrap();
        let dot_flox = CanonicalPath::new(dir.path()).unwrap();
        assert_eq!(ensure(&dot_flox), Definition::New);
        let first = read(&dot_flox);
        assert_eq!(
            ensure(&dot_flox),
            Definition::Existing,
            "a second ensure reports the definition already existed"
        );
        assert_eq!(
            read(&dot_flox),
            first,
            "ensuring again keeps the existing id"
        );
    }

    #[test]
    fn malformed_file_reads_as_absent() {
        let dir = tempdir().unwrap();
        let dot_flox = CanonicalPath::new(dir.path()).unwrap();
        std::fs::write(dir.path().join(TELEMETRY_ID_FILENAME), "not-a-uuid").unwrap();
        assert_eq!(read(&dot_flox), None);
    }
}
