//! The local environment's stable id, stored as a bare UUID string in
//! `.flox/telemetry_id`.
//!
//! Minted in the CLI layer when a path environment is created (`flox init`,
//! `flox pull --copy`) and read at event-emit time to populate
//! `local_environment_id`. It lives in the committed `.flox` alongside
//! `env.json`, so it travels with a git-clone or folder-copy, but it is
//! deliberately not part of the environment pointer, and the SDK is not
//! involved in minting or reading it.

use std::path::Path;

use flox_core::write_atomically;
use tracing::debug;
use uuid::Uuid;

/// File inside `.flox` holding the environment's stable local id.
pub(crate) const TELEMETRY_ID_FILENAME: &str = "telemetry_id";

/// Read the local environment id from `<dot_flox>/telemetry_id`. Best-effort
/// and read-only: a missing or malformed file yields `None`, never an error,
/// and reading never writes.
pub(crate) fn read(dot_flox: &Path) -> Option<Uuid> {
    let contents = std::fs::read_to_string(dot_flox.join(TELEMETRY_ID_FILENAME)).ok()?;
    Uuid::try_parse(contents.trim()).ok()
}

/// Mint a stable local id for a newly created path environment and write it to
/// `<dot_flox>/telemetry_id`, returning it. Idempotent: if a valid id already
/// exists it is returned unchanged, so calling this on an already-identified
/// environment never regenerates the id. Best-effort: a write failure is
/// logged, and that environment then carries no id for its lifetime (reads
/// never write, so nothing recreates it).
pub(crate) fn mint(dot_flox: &Path) -> Uuid {
    if let Some(existing) = read(dot_flox) {
        return existing;
    }
    let id = Uuid::new_v4();
    if let Err(err) = write_atomically(dot_flox.join(TELEMETRY_ID_FILENAME), format!("{id}\n")) {
        debug!(error = %err, "could not write local_environment_id");
    }
    id
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn mint_then_read_round_trips() {
        let dir = tempdir().unwrap();
        assert_eq!(read(dir.path()), None, "no id before minting");
        let id = mint(dir.path());
        assert_eq!(read(dir.path()), Some(id), "read returns the minted id");
    }

    #[test]
    fn mint_is_idempotent() {
        let dir = tempdir().unwrap();
        let first = mint(dir.path());
        assert_eq!(
            mint(dir.path()),
            first,
            "a second mint keeps the existing id"
        );
    }

    #[test]
    fn malformed_file_reads_as_absent() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(TELEMETRY_ID_FILENAME), "not-a-uuid").unwrap();
        assert_eq!(read(dir.path()), None);
    }
}
