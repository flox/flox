//! Rate-limited warning for unauthenticated catalog resolution.
//!
//! [`FloxhubClient::resolve`](floxhub_client::FloxhubClient::resolve) invokes
//! the hook built here when it is about to contact the catalog `/resolve`
//! endpoint without authentication material — the exact call that will fail
//! once catalog auth gating is enforced server-side. Warning at that point
//! (rather than per command) means users see the warning precisely where they
//! will later see the failure: a fully locked environment resolves nothing
//! and stays quiet, while the first lock of an environment warns regardless
//! of which command triggered it.

use std::path::Path;
use std::{fs, io};

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use tracing::debug;

use crate::utils::message;

const RESOLVE_AUTH_WARNING_FILE_NAME: &str = "resolve-auth-warning-timestamp.json";
const RESOLVE_AUTH_WARNING_EXPIRY: Duration = Duration::hours(8);

// TODO(DEV-200): append the docs URL explaining the auth transition once the
// explainer page exists.
const RESOLVE_AUTH_WARNING: &str = "Resolving packages will require authentication to FloxHub in an upcoming release.\nRun 'flox auth login' to authenticate now.";

/// Timestamp serialized to a file to track when the user was last warned.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct LastResolveAuthWarning {
    #[serde(with = "time::serde::iso8601")]
    last_warning: OffsetDateTime,
}

/// Warn that resolution will require authentication, at most once per
/// [`RESOLVE_AUTH_WARNING_EXPIRY`] per user (tracked by a timestamp file in
/// the cache directory).
///
/// Emitted via [`message::warning`], so `-q` silences it through the logger
/// filter and it lands on stderr like every other advisory message.
pub(crate) fn warn_unauthenticated_resolve(cache_dir: impl AsRef<Path>) {
    let stamp_file = cache_dir.as_ref().join(RESOLVE_AUTH_WARNING_FILE_NAME);
    let now = OffsetDateTime::now_utc();
    if !should_warn(&stamp_file, now) {
        return;
    }
    message::warning(RESOLVE_AUTH_WARNING);
    record_warning(&stamp_file, now);
}

/// True when the stamp file is absent, unreadable, invalid, or older than
/// [`RESOLVE_AUTH_WARNING_EXPIRY`]. Failures count as "warn": losing the
/// stamp only repeats the warning.
fn should_warn(stamp_file: &Path, now: OffsetDateTime) -> bool {
    let contents = match fs::read_to_string(stamp_file) {
        Ok(contents) => contents,
        Err(err) => {
            if err.kind() != io::ErrorKind::NotFound {
                debug!(%err, "failed to read resolve-auth-warning stamp");
            }
            return true;
        },
    };
    match serde_json::from_str::<LastResolveAuthWarning>(&contents) {
        Ok(stamp) => now - stamp.last_warning >= RESOLVE_AUTH_WARNING_EXPIRY,
        Err(err) => {
            debug!(%err, "invalid resolve-auth-warning stamp");
            true
        },
    }
}

/// Best-effort write of the stamp file; failure only means the warning may
/// repeat sooner than the expiry.
fn record_warning(stamp_file: &Path, now: OffsetDateTime) {
    let stamp = LastResolveAuthWarning { last_warning: now };
    let contents = match serde_json::to_string(&stamp) {
        Ok(contents) => contents,
        Err(err) => {
            debug!(%err, "failed to serialize resolve-auth-warning stamp");
            return;
        },
    };
    if let Err(err) = fs::write(stamp_file, contents) {
        debug!(%err, "failed to write resolve-auth-warning stamp");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp_in(dir: &Path) -> std::path::PathBuf {
        dir.join(RESOLVE_AUTH_WARNING_FILE_NAME)
    }

    /// The backdated stamp written by catalog-auth-warnings.bats must parse
    /// as a valid, expired timestamp — not fall into the invalid-stamp path.
    #[test]
    fn bats_backdated_stamp_parses_as_expired() {
        let dir = tempfile::tempdir().unwrap();
        let stamp_file = stamp_in(dir.path());
        fs::write(&stamp_file, r#"{"last_warning":"2020-01-01T00:00:00Z"}"#).unwrap();
        let stamp: LastResolveAuthWarning =
            serde_json::from_str(&fs::read_to_string(&stamp_file).unwrap())
                .expect("bats fixture timestamp must parse");
        assert_eq!(stamp.last_warning.year(), 2020);
        assert!(should_warn(&stamp_file, OffsetDateTime::now_utc()));
    }

    #[test]
    fn warns_when_stamp_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(should_warn(
            &stamp_in(dir.path()),
            OffsetDateTime::now_utc()
        ));
    }

    #[test]
    fn does_not_warn_within_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let stamp_file = stamp_in(dir.path());
        let now = OffsetDateTime::now_utc();
        record_warning(&stamp_file, now);
        assert!(!should_warn(&stamp_file, now));
        assert!(!should_warn(
            &stamp_file,
            now + RESOLVE_AUTH_WARNING_EXPIRY - Duration::seconds(1)
        ));
    }

    #[test]
    fn warns_again_after_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let stamp_file = stamp_in(dir.path());
        let now = OffsetDateTime::now_utc();
        record_warning(&stamp_file, now);
        assert!(should_warn(&stamp_file, now + RESOLVE_AUTH_WARNING_EXPIRY));
    }

    #[test]
    fn warns_when_stamp_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let stamp_file = stamp_in(dir.path());
        fs::write(&stamp_file, "not json").unwrap();
        assert!(should_warn(&stamp_file, OffsetDateTime::now_utc()));
    }

    #[test]
    fn warn_unauthenticated_resolve_writes_stamp() {
        let dir = tempfile::tempdir().unwrap();
        warn_unauthenticated_resolve(dir.path());
        assert!(!should_warn(
            &stamp_in(dir.path()),
            OffsetDateTime::now_utc()
        ));
    }
}
