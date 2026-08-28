//! The local environment's stable id, stored in the committed
//! `.flox/env.json` (see [EnvJson]) so it travels with a git-clone or
//! folder-copy.
//!
//! Minted when a path environment is created (`flox init`, `flox pull
//! --copy`) and read at event-emit time to populate `local_environment_id`.
//!
//! Flox 1.14.1 minted into a separate `.flox/telemetry_id` file, still read
//! as a fallback but never written. Only a version-control merge can leave a
//! checkout with both sources disagreeing; this version then reports the
//! `env.json` id and 1.14.1 the file's.

use flox_core::data::CanonicalPath;
use flox_rust_sdk::models::environment::EnvJson;
use tracing::debug;
use uuid::Uuid;

/// Holds a bare UUID string, the format flox 1.14.1 wrote.
pub(crate) const TELEMETRY_ID_FILENAME: &str = "telemetry_id";

fn read_legacy(dot_flox: &CanonicalPath) -> Option<Uuid> {
    let contents = std::fs::read_to_string(dot_flox.join(TELEMETRY_ID_FILENAME)).ok()?;
    Uuid::try_parse(contents.trim()).ok()
}

/// Read the local environment id. Read-only by design: a malformed or
/// missing value stays `None` rather than being re-minted.
pub(crate) fn read(dot_flox: &CanonicalPath) -> Option<Uuid> {
    EnvJson::read_from(dot_flox)
        .ok()
        .and_then(|env_json| env_json.env_id)
        .or_else(|| read_legacy(dot_flox))
}

/// Where the id at a `.flox` came from. `flox pull --copy` converts in
/// place, but only a legacy `telemetry_id` file survives `flox push` — a
/// pushed `env_id` is stripped with the rest of `env.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// Nothing was there, so this call minted one. A failed write still
    /// reports `Minted` — the environment is new, it just carries no id.
    Minted,
    /// An id was already present and was kept.
    Existing,
}

/// Ensure a newly created path environment has a stable local id.
/// Idempotent: an existing id in either location is kept. A failed write is
/// logged and leaves that environment without an id for its lifetime, since
/// only creation mints.
pub(crate) fn ensure(dot_flox: &CanonicalPath) -> Origin {
    let env_json = EnvJson::read_from(dot_flox).ok();
    let existing_id = env_json
        .as_ref()
        .and_then(|env_json| env_json.env_id)
        .or_else(|| read_legacy(dot_flox));
    if existing_id.is_some() {
        return Origin::Existing;
    }
    let Some(mut env_json) = env_json else {
        debug!("could not read or parse env.json, environment will carry no local id");
        return Origin::Minted;
    };
    env_json.env_id = Some(Uuid::new_v4());
    if let Err(err) = env_json.write_to(dot_flox) {
        debug!(error = %err, "could not write local_environment_id");
    }
    Origin::Minted
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use flox_core::data::environment_ref::EnvironmentName;
    use flox_rust_sdk::models::environment::{
        ENVIRONMENT_POINTER_FILENAME,
        EnvironmentPointer,
        PathPointer,
    };
    use indoc::indoc;
    use tempfile::tempdir;

    use super::*;

    /// The pointer half of the `env.json` written by [dot_flox_with_env_json].
    fn test_pointer() -> EnvironmentPointer {
        EnvironmentPointer::Path(PathPointer::new(EnvironmentName::from_str("test").unwrap()))
    }

    /// A `.flox` as the creation sites leave it by the time `ensure` runs.
    fn dot_flox_with_env_json() -> (tempfile::TempDir, CanonicalPath) {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(ENVIRONMENT_POINTER_FILENAME), indoc! {r#"
                {
                  "name": "test",
                  "version": 1
                }
            "#})
        .unwrap();
        let dot_flox = CanonicalPath::new(dir.path()).unwrap();
        (dir, dot_flox)
    }

    #[test]
    fn ensure_mints_into_env_json() {
        let (dir, dot_flox) = dot_flox_with_env_json();
        assert_eq!(read(&dot_flox), None, "no id before minting");
        assert_eq!(ensure(&dot_flox), Origin::Minted);

        let minted = read(&dot_flox);
        assert_ne!(minted, None, "env.json carries the minted id");
        assert_eq!(EnvJson::read_from(&dot_flox).unwrap(), EnvJson {
            pointer: test_pointer(),
            env_id: minted,
        });
        assert!(
            !dir.path().join(TELEMETRY_ID_FILENAME).exists(),
            "the legacy file is never written"
        );
    }

    #[test]
    fn ensure_keeps_existing_id() {
        let (_dir, dot_flox) = dot_flox_with_env_json();
        assert_eq!(ensure(&dot_flox), Origin::Minted);
        let first = read(&dot_flox);
        assert_eq!(
            ensure(&dot_flox),
            Origin::Existing,
            "a second ensure reports the id was already there"
        );
        assert_eq!(
            read(&dot_flox),
            first,
            "ensuring again keeps the existing id"
        );
    }

    #[test]
    fn read_falls_back_to_legacy_telemetry_id_file() {
        let (dir, dot_flox) = dot_flox_with_env_json();
        let legacy_id = Uuid::new_v4();
        std::fs::write(
            dir.path().join(TELEMETRY_ID_FILENAME),
            format!("{legacy_id}\n"),
        )
        .unwrap();
        assert_eq!(read(&dot_flox), Some(legacy_id));
    }

    #[test]
    fn legacy_id_counts_as_existing_and_is_not_migrated() {
        let (dir, dot_flox) = dot_flox_with_env_json();
        std::fs::write(
            dir.path().join(TELEMETRY_ID_FILENAME),
            format!("{}\n", Uuid::new_v4()),
        )
        .unwrap();
        assert_eq!(
            ensure(&dot_flox),
            Origin::Existing,
            "legacy id counts as existing"
        );

        assert_eq!(
            EnvJson::read_from(&dot_flox).unwrap(),
            EnvJson {
                pointer: test_pointer(),
                env_id: None,
            },
            "an existing legacy id is not migrated into env.json"
        );
    }

    #[test]
    fn env_json_id_wins_over_legacy_file() {
        let (dir, dot_flox) = dot_flox_with_env_json();
        assert_eq!(ensure(&dot_flox), Origin::Minted);
        let minted = read(&dot_flox);
        assert_ne!(minted, None);

        std::fs::write(
            dir.path().join(TELEMETRY_ID_FILENAME),
            format!("{}\n", Uuid::new_v4()),
        )
        .unwrap();
        assert_eq!(read(&dot_flox), minted, "env.json id takes precedence");
    }

    #[cfg(unix)]
    #[test]
    fn ensure_preserves_env_json_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, dot_flox) = dot_flox_with_env_json();
        let env_json_path = dir.path().join(ENVIRONMENT_POINTER_FILENAME);
        std::fs::set_permissions(&env_json_path, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(ensure(&dot_flox), Origin::Minted);

        let mode = std::fs::metadata(&env_json_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o644, "minting keeps env.json world-readable");
    }

    #[test]
    fn malformed_env_id_reads_as_absent() {
        let (dir, dot_flox) = dot_flox_with_env_json();
        std::fs::write(dir.path().join(ENVIRONMENT_POINTER_FILENAME), indoc! {r#"
                {
                  "name": "test",
                  "version": 1,
                  "env_id": "not-a-uuid"
                }
            "#})
        .unwrap();
        assert_eq!(read(&dot_flox), None);
    }

    #[test]
    fn malformed_legacy_file_reads_as_absent() {
        let (dir, dot_flox) = dot_flox_with_env_json();
        std::fs::write(dir.path().join(TELEMETRY_ID_FILENAME), "not-a-uuid").unwrap();
        assert_eq!(read(&dot_flox), None);
    }

    #[test]
    fn ensure_without_env_json_mints_nothing() {
        let dir = tempdir().unwrap();
        let dot_flox = CanonicalPath::new(dir.path()).unwrap();
        assert_eq!(
            ensure(&dot_flox),
            Origin::Minted,
            "a missing env.json still reports a new environment"
        );
        assert_eq!(read(&dot_flox), None, "but no id could be stored");
    }
}
