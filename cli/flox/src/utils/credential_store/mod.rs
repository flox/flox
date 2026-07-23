//! Storage backend abstraction for the FloxHub auth token.
//!
//! The token's runtime representation (`Option<String>` in [Config] → parsed
//! `FloxhubToken` → `AuthContext`) is unchanged; this module owns only the
//! *source* of the string and the *destination* of writes.
//!
//! The abstraction follows the `enum_dispatch` + `Mock`-arm pattern used by
//! `InstallableLockerImpl`
//! (`cli/flox-rust-sdk/src/providers/flake_installable_locker.rs`), and the
//! typed-error convention used by `AuthError`
//! (`cli/flox-rust-sdk/src/providers/nix_auth.rs`): any "no backend" or
//! credential-redaction concern lives in the error type rather than at call
//! sites.
//!
//! Each backend lives in its own file — [keyring], [plaintext], and [mock] —
//! while this module holds the shared [CredentialStore] trait, the error type,
//! and the [CredentialStores] orchestration.

mod keyring;
mod mock;
mod plaintext;

use std::path::{Path, PathBuf};

use enum_dispatch::enum_dispatch;
use flox_config::{Config, FLOX_CONFIG_FILE, TokenStorageMode};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::floxmeta::FLOXHUB_TOKEN_ENV_VAR;
use indoc::indoc;
pub use keyring::KeyringStore;
pub use mock::MockStore;
pub use plaintext::PlaintextStore;
use thiserror::Error;
use url::Url;

use crate::utils::message;

/// Errors returned by a [CredentialStore].
///
/// Per the project conventions, credential redaction and backend-availability
/// classification belong here rather than at call sites. The underlying writes
/// (`update_config`) never interpolate the token into their messages, so no
/// variant carries the secret.
#[derive(Debug, Error)]
pub enum CredentialStoreError {
    /// A read or write against the plaintext `flox.toml` failed.
    #[error("could not access the plaintext credential file")]
    Plaintext(#[source] anyhow::Error),

    /// Failed to set owner-only permissions on the plaintext credential file.
    #[error("could not set permissions on the plaintext credential file")]
    SetPermissions(#[source] std::io::Error),

    /// Could not read the plaintext credential file to probe its contents.
    #[error("could not read the plaintext credential file")]
    ReadPlaintext(#[source] std::io::Error),

    /// Could not parse the plaintext credential file as TOML.
    #[error("could not parse the plaintext credential file")]
    ParsePlaintext(#[source] toml_edit::TomlError),

    /// No usable OS keyring backend is available (no default store, platform
    /// failure, or the store could not be accessed). Callers treat this as the
    /// signal to fall back to plaintext storage. The underlying keyring error
    /// never carries the secret.
    #[error("no OS keyring backend is available")]
    NoBackend(#[source] keyring_core::Error),

    /// An unclassified OS keyring failure that is neither "no entry" nor a
    /// known no-backend condition.
    #[error("the OS keyring operation failed")]
    Keyring(#[source] keyring_core::Error),

    /// The OS keyring is disabled via `_FLOX_DISABLE_KEYRING`. Treated like a
    /// no-backend condition: writes fail so callers fall back to plaintext, and
    /// no real keyring backend is ever initialized. Used by the test suite to
    /// keep integration tests off the developer's global OS keyring.
    #[error("the OS keyring is disabled")]
    Disabled,

    /// An error injected by [MockStore] for testing.
    #[error("{0}")]
    Mock(String),
}

/// Where the active FloxHub credential came from.
///
/// Determined from the underlying primitives (env var, user file, merged
/// config) rather than the already-merged config value, so the same probe can
/// distinguish a system-config token from a user-file token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    /// `FLOX_FLOXHUB_TOKEN` is set in the environment.
    Env,
    /// The token came from the system config (`/etc/flox.toml`).
    SystemConfig,
    /// The token is stored in plain text in the user's `flox.toml`.
    UserConfigPlaintext,
    /// The token is stored in the OS keyring.
    Keyring,
    /// No credential is available from any source.
    None,
}

impl CredentialSource {
    /// The shared "stored in plain text at `<path>`" sentence.
    ///
    /// Used both by `flox auth status` (the [CredentialSource::UserConfigPlaintext]
    /// line) and by the plaintext-fallback warning at login, so the wording stays
    /// identical in both places.
    pub(crate) fn plaintext_notice(plaintext_path: &Path) -> String {
        format!(
            "Credential stored in plain text at '{}'.",
            plaintext_path.display()
        )
    }

    /// The user-facing line for `flox auth status` describing where the active
    /// credential is stored, or `None` when there is no line to show.
    ///
    /// `SystemConfig` and `None` produce no line: the former is an
    /// administrator-provided token the user cannot relocate, and the latter
    /// means there is nothing stored.
    pub fn describe_storage(&self, plaintext_path: &Path) -> Option<String> {
        match self {
            CredentialSource::UserConfigPlaintext => Some(Self::plaintext_notice(plaintext_path)),
            CredentialSource::Keyring => {
                Some("Credential stored in your system keyring.".to_string())
            },
            CredentialSource::Env => Some(
                "Credential read from the FLOX_FLOXHUB_TOKEN environment variable.".to_string(),
            ),
            CredentialSource::SystemConfig | CredentialSource::None => None,
        }
    }
}

/// Storage backend for the FloxHub auth token.
#[enum_dispatch]
pub trait CredentialStore {
    /// Return the stored token, or `None` when this backend has no token.
    fn get(&self) -> Result<Option<String>, CredentialStoreError>;
    /// Store `token`, replacing any previously stored value.
    fn set(&self, token: &str) -> Result<(), CredentialStoreError>;
    /// Remove the stored token. Idempotent: succeeds when nothing is stored.
    fn remove(&self) -> Result<(), CredentialStoreError>;
}

/// The concrete credential backends.
#[enum_dispatch(CredentialStore)]
#[derive(Debug, Clone)]
pub enum CredentialStoreImpl {
    /// OS-native encrypted credential store (macOS Keychain / Linux Secret
    /// Service), keyed by the FloxHub base URL.
    Keyring(KeyringStore),
    /// `<config_dir>/flox.toml`, with an explicit `0600` on write.
    Plaintext(PlaintextStore),
    /// In-memory backend for tests; supports injected errors.
    Mock(MockStore),
}

/// Where a freshly-logged-in token was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenStorage {
    /// Stored in the OS keyring.
    Keyring,
    /// Stored in the plaintext `flox.toml` file (the fallback).
    Plaintext,
}

/// What the upstream resolver did, so the caller can emit the right message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// An existing plaintext token was moved into the keyring and removed from
    /// the plaintext file. The caller emits the one-time migration note.
    Migrated,
    /// The token was written to the keyring, but the plaintext copy could not be
    /// removed (e.g. an unwritable `flox.toml`). The keyring now holds the token,
    /// but the plaintext secret lingers and shadows it (user file > keyring), so
    /// the caller warns the user rather than letting this fail silently.
    MigratedButPlaintextRemains,
    /// `config.flox.floxhub_token` was empty and was populated from the keyring.
    PopulatedFromKeyring,
    /// Nothing changed (env/system token present, no plaintext to migrate, or
    /// the keyring is empty).
    Unchanged,
}

/// The keyring + plaintext credential-store pair for a single FloxHub instance.
///
/// The two backends are derived once from the FloxHub base URL and the user
/// config directory, then shared by every credential operation — login,
/// logout, `status`, and the startup resolver — so the inputs have a single
/// derivation point and no call site rebuilds its own bare stores.
#[derive(Debug, Clone)]
pub struct CredentialStores {
    keyring: CredentialStoreImpl,
    plaintext: CredentialStoreImpl,
    /// The user config directory, retained for user-facing messages that name
    /// the plaintext file path.
    config_dir: PathBuf,
}

impl CredentialStores {
    /// Build the store pair from the FloxHub base URL and the user config
    /// directory.
    ///
    /// Used at startup, before a [Flox] exists; [Self::from_flox] is the
    /// convenience for the command handlers that already hold one.
    pub fn new(floxhub_url: &Url, config_dir: impl Into<PathBuf>) -> Self {
        let config_dir = config_dir.into();
        Self {
            keyring: CredentialStoreImpl::Keyring(KeyringStore::new(floxhub_url)),
            plaintext: CredentialStoreImpl::Plaintext(PlaintextStore::new(config_dir.clone())),
            config_dir,
        }
    }

    /// Build the store pair from a [Flox], using its FloxHub base URL and
    /// config directory.
    pub fn from_flox(flox: &Flox) -> Self {
        Self::new(flox.floxhub.base_url(), &flox.config_dir)
    }

    /// Path to the plaintext `flox.toml`, for user-facing messages about where
    /// a plaintext credential lives.
    pub fn plaintext_path(&self) -> PathBuf {
        self.config_dir.join(FLOX_CONFIG_FILE)
    }

    /// Determine where the active FloxHub credential comes from.
    ///
    /// Pure: no migration or other side effects. Shared by the startup resolver
    /// and `flox auth status`.
    ///
    /// Precedence: `FLOX_FLOXHUB_TOKEN` env > user-file plaintext > keyring >
    /// system config > none.
    ///
    /// The keyring branch is value-aware: the keyring is the source only when
    /// the merged `config.flox.floxhub_token` is empty (the resolver would
    /// populate it from the keyring) or equals the keyring entry (the resolver
    /// already did). A non-empty merged token that differs from the keyring
    /// entry is not being read from the keyring at all — it came from
    /// `/etc/flox.toml` — and must report `SystemConfig` even when the keyring
    /// also holds an unrelated entry, so an invalid system token never routes
    /// [Self::clear_invalid] to the user's saved keyring credential.
    pub fn probe_source(&self, config: &Config) -> CredentialSource {
        let env_token = std::env::var(FLOXHUB_TOKEN_ENV_VAR).ok();
        if env_token.is_some_and(|t| !t.is_empty()) {
            return CredentialSource::Env;
        }

        if self.plaintext.get().ok().flatten().is_some() {
            return CredentialSource::UserConfigPlaintext;
        }

        let merged_token = config
            .flox
            .floxhub_token
            .as_deref()
            .filter(|t| !t.is_empty());

        if let Ok(Some(keyring_token)) = self.keyring.get()
            && merged_token.is_none_or(|t| t == keyring_token)
        {
            return CredentialSource::Keyring;
        }

        // The merged config still has a token, but it is not from the
        // environment, the user file, or the keyring — so it came from
        // `/etc/flox.toml`.
        if merged_token.is_some() {
            return CredentialSource::SystemConfig;
        }

        CredentialSource::None
    }

    /// Persist a logged-in token according to `target`.
    ///
    /// `Keyring`: attempt the keyring first (try-then-confirm); on success
    /// store there and remove any lingering plaintext token so it cannot
    /// shadow the keyring entry, and on any keyring failure fall back to the
    /// plaintext file (`0600`). `Plaintext`: write the plaintext file and drop
    /// any existing keyring entry (best effort). The returned [TokenStorage]
    /// tells the caller whether to warn the user.
    pub fn persist_login_token(
        &self,
        token: &str,
        target: TokenStorageMode,
    ) -> Result<TokenStorage, CredentialStoreError> {
        if target == TokenStorageMode::Keyring && self.keyring.set(token).is_ok() {
            // The keyring already holds the token, so a failure to remove the
            // old plaintext copy must not fail the login. Warn instead: a
            // lingering plaintext token both leaves a secret on disk and shadows
            // the keyring on the next read (user file > keyring).
            if let Err(e) = self.plaintext.remove() {
                tracing::warn!(
                    error = %e,
                    "could not remove the plaintext credential after a keyring write"
                );
                message::warning(indoc! {"
                    Stored your credential in the system keyring.
                    The existing plain-text credential in flox.toml could not be removed.
                    Remove 'floxhub_token' from flox.toml so it does not shadow the keyring."});
            }
            return Ok(TokenStorage::Keyring);
        }

        self.plaintext.set(token)?;
        // An explicit plain-text choice supersedes any keyring entry: drop a
        // lingering keyring token (best effort) so it is not left behind as a
        // stale secret, and is not surfaced on a later read if the plain-text
        // file is removed. (The plain-text file already takes read precedence
        // over the keyring, so this is cleanup, not shadowing.) Scoped to the
        // explicit `Plaintext` target — on a keyring-write fallback there is
        // nothing of ours in the keyring to remove.
        if target == TokenStorageMode::Plaintext
            && let Err(e) = self.keyring.remove()
        {
            tracing::debug!(
                error = %e,
                "could not remove the keyring credential after storing plain text"
            );
        }
        Ok(TokenStorage::Plaintext)
    }

    /// Resolve the FloxHub credential for this invocation: opportunistically
    /// migrate an existing plaintext token into the keyring, and populate
    /// `config.flox.floxhub_token` from the keyring when the merged config
    /// supplied no token.
    ///
    /// This is the single upstream resolution step (Q8, Option C): both the loud
    /// `resolve_auth_context` and the silent `init_floxhub_client` read the same
    /// field, so resolving it once here lets both see the keyring value with no
    /// change to their internals.
    ///
    /// The caller gates this on "Auth0 mode, outside the prompt/hook flow": in
    /// other modes (e.g. Kerberos) the token is not used for authentication, so
    /// a legacy `floxhub_token` left in `flox.toml` must not be silently moved or
    /// read, and the prompt/hook flow does no keyring I/O. This method assumes
    /// that gate has already passed and performs the I/O unconditionally.
    ///
    /// Migration (additive over Phase 2) runs only when all of these hold, so it
    /// is correct rather than merely convenient:
    /// - `FLOX_FLOXHUB_TOKEN` is not present in the environment — a transient
    ///   CI token is never persisted, and an explicit *empty* export (used to
    ///   mask saved credentials for one invocation) blocks the stores entirely.
    /// - the *user file* (`PlaintextStore::get`) holds a token — the system
    ///   `/etc/flox.toml` token never appears here, so it is never migrated.
    /// - the storage preference (`config.flox.floxhub_token_storage`) is
    ///   `Keyring` — when the user has chosen plain-text storage, a plaintext
    ///   token is left in place rather than moved.
    ///
    /// The migration moves the token store-to-store only; it never writes
    /// `config.flox.floxhub_token`. The merge already populated that field with
    /// the user-file value, and rewriting it would corrupt precedence in the
    /// env-unset / system-token edge case (where the user-file token differs from
    /// the merged system token) and would disturb the loud/silent dual-parse this
    /// same invocation performs.
    ///
    /// On any keyring failure the plaintext file is left untouched: no data loss,
    /// no migration. Precedence is otherwise preserved by only consulting the
    /// keyring for a *read* when the merged value (env > user file > system) is
    /// empty.
    pub fn resolve_into(&self, config: &mut Config) -> ResolveOutcome {
        // Any explicit `FLOX_FLOXHUB_TOKEN` — including an *empty* export used
        // to mask saved credentials for one invocation — takes precedence over
        // both stores: never migrate the plaintext token, and never populate
        // the config from the keyring. Presence is what matters here; whether
        // the value is a usable token is validated downstream.
        if std::env::var(FLOXHUB_TOKEN_ENV_VAR).is_ok() {
            return ResolveOutcome::Unchanged;
        }

        // Opportunistic migration: only the user-file token is eligible, and
        // only when the standing preference is keyring storage — a chosen
        // plain-text token is left in place rather than moved. Probe the
        // plaintext file directly (provenance-aware) rather than trusting the
        // merged config field, which may hold a system token instead.
        if config.flox.floxhub_token_storage == TokenStorageMode::Keyring
            && let Ok(Some(token)) = self.plaintext.get()
        {
            // Try-then-confirm: only after the keyring write succeeds do we
            // remove the plaintext token. On any keyring failure the plaintext
            // file is left exactly as it was.
            if self.keyring.set(&token).is_ok() {
                return match self.plaintext.remove() {
                    Ok(()) => ResolveOutcome::Migrated,
                    // The keyring now holds the token, so this is not a no-op: do
                    // not return `Unchanged` (which would be silent and re-attempt
                    // every command). Report the partial migration so the caller
                    // warns; the lingering plaintext token still shadows the
                    // keyring.
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "could not remove the plaintext credential after migrating it to the keyring"
                        );
                        ResolveOutcome::MigratedButPlaintextRemains
                    },
                };
            }
            return ResolveOutcome::Unchanged;
        }

        if config
            .flox
            .floxhub_token
            .as_deref()
            .is_some_and(|t| !t.is_empty())
        {
            return ResolveOutcome::Unchanged;
        }

        if let Ok(Some(token)) = self.keyring.get() {
            config.flox.floxhub_token = Some(token);
            return ResolveOutcome::PopulatedFromKeyring;
        }

        ResolveOutcome::Unchanged
    }

    /// Remove an invalid FloxHub credential from the store that actually
    /// supplied it, identified by `source`.
    ///
    /// The startup resolver discovers an invalid token only after it has been
    /// merged into the config, so the bad token may come from any source. Only
    /// clear a store we own and that actually provided the token:
    /// - [CredentialSource::Keyring] → clear the keyring.
    /// - [CredentialSource::UserConfigPlaintext] → clear the plaintext file.
    /// - [CredentialSource::Env] / [CredentialSource::SystemConfig] /
    ///   [CredentialSource::None] → clear nothing. The invalid value came from
    ///   `FLOX_FLOXHUB_TOKEN` or `/etc/flox.toml` (or nowhere), so deleting a
    ///   valid saved keyring/plaintext credential would force a needless re-login
    ///   once the env/system value is corrected.
    ///
    /// Removals are idempotent, so this is safe regardless of provenance.
    pub fn clear_invalid(&self, source: CredentialSource) {
        match source {
            CredentialSource::Keyring => {
                if let Err(e) = self.keyring.remove() {
                    tracing::debug!(error = %e, "could not remove invalid token from the keyring");
                }
            },
            CredentialSource::UserConfigPlaintext => {
                if let Err(e) = self.plaintext.remove() {
                    tracing::debug!(error = %e, "could not remove invalid token from the plaintext file");
                }
            },
            // The invalid value came from `FLOX_FLOXHUB_TOKEN`, `/etc/flox.toml`,
            // or nowhere — none of which we own. Leave any saved credential
            // intact.
            CredentialSource::Env | CredentialSource::SystemConfig | CredentialSource::None => {},
        }
    }

    /// Remove the token from both stores, for logout.
    ///
    /// The resolver may have populated the token from the keyring, and a
    /// plaintext token may also linger from before migration, so both are
    /// cleared. Both removals are idempotent, so this succeeds when nothing is
    /// stored.
    ///
    /// Both removals are always attempted: a keyring platform error (e.g. a
    /// locked Secret Service session) must not short-circuit logout and leave
    /// the plaintext secret on disk. A plaintext failure is reported first —
    /// that is the copy sitting in a file.
    pub fn remove_all(&self) -> Result<(), CredentialStoreError> {
        let keyring_result = self.keyring.remove();
        self.plaintext.remove()?;
        keyring_result
    }
}

/// Helpers shared by this module's tests and the per-backend tests in
/// [keyring], [plaintext], and [mock].
#[cfg(test)]
pub(crate) mod test_helpers {
    use std::path::Path;

    use flox_config::FLOX_CONFIG_FILE;

    /// An opaque token. The store reads/writes arbitrary strings; the probe is
    /// presence-based. Neither needs a JWT-shaped value.
    pub(crate) const TOKEN: &str = "opaque-token-value";

    /// Write a `flox.toml` with the given contents into `dir`.
    pub(crate) fn write_flox_toml(dir: &Path, contents: &str) {
        std::fs::write(dir.join(FLOX_CONFIG_FILE), contents).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use pretty_assertions::assert_eq;

    use super::test_helpers::{TOKEN, write_flox_toml};
    use super::*;

    impl CredentialStores {
        /// Assemble the pair from arbitrary backends (typically [MockStore]) so
        /// the orchestration methods can be exercised without a real keyring or
        /// FloxHub URL. `config_dir` is left empty because these tests do not
        /// exercise the path-bearing messages.
        fn from_stores(keyring: CredentialStoreImpl, plaintext: CredentialStoreImpl) -> Self {
            Self {
                keyring,
                plaintext,
                config_dir: PathBuf::new(),
            }
        }
    }

    // --- CredentialStores::probe_source: the four Phase 1 input shapes ---
    //
    // Driven through the public `Config::parse()` under `temp_env::with_vars`,
    // mirroring `test_set_by_env` (config/mod.rs:555): `HOME`,
    // `FLOX_CONFIG_DIR` (user dir), and `FLOX_SYSTEM_CONFIG_DIR` (system dir)
    // are set so parsing is hermetic.

    /// Common env scaffolding for a probe test.
    fn probe_vars<'a>(
        home: &'a Path,
        user_dir: &'a Path,
        system_dir: &'a Path,
        floxhub_token: Option<&'a str>,
    ) -> Vec<(&'a str, Option<&'a str>)> {
        vec![
            ("HOME", Some(home.to_str().unwrap())),
            ("FLOX_CONFIG_DIR", Some(user_dir.to_str().unwrap())),
            ("FLOX_SYSTEM_CONFIG_DIR", Some(system_dir.to_str().unwrap())),
            ("FLOX_FLOXHUB_TOKEN", floxhub_token),
        ]
    }

    #[test]
    fn probe_returns_none_when_no_token_anywhere() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();
        let system_dir = tempfile::tempdir().unwrap();
        write_flox_toml(user_dir.path(), "");
        write_flox_toml(system_dir.path(), "");

        temp_env::with_vars(
            probe_vars(home.path(), user_dir.path(), system_dir.path(), None),
            || {
                let config = Config::parse().unwrap();
                let plaintext =
                    CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                let keyring = CredentialStoreImpl::Mock(MockStore::new());
                let stores = CredentialStores::from_stores(keyring, plaintext);
                assert_eq!(stores.probe_source(&config), CredentialSource::None);
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    #[test]
    fn probe_returns_env_when_env_var_set() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();
        let system_dir = tempfile::tempdir().unwrap();
        // A user-file token is also present to prove env wins over it.
        write_flox_toml(user_dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
        write_flox_toml(system_dir.path(), "");

        temp_env::with_vars(
            probe_vars(
                home.path(),
                user_dir.path(),
                system_dir.path(),
                Some("env-token"),
            ),
            || {
                let config = Config::parse().unwrap();
                let plaintext =
                    CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                let keyring = CredentialStoreImpl::Mock(MockStore::new());
                let stores = CredentialStores::from_stores(keyring, plaintext);
                assert_eq!(stores.probe_source(&config), CredentialSource::Env);
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    #[test]
    fn probe_returns_user_config_plaintext_for_user_file_token() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();
        let system_dir = tempfile::tempdir().unwrap();
        write_flox_toml(user_dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
        write_flox_toml(system_dir.path(), "");

        temp_env::with_vars(
            probe_vars(home.path(), user_dir.path(), system_dir.path(), None),
            || {
                let config = Config::parse().unwrap();
                let plaintext =
                    CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                let keyring = CredentialStoreImpl::Mock(MockStore::new());
                let stores = CredentialStores::from_stores(keyring, plaintext);
                assert_eq!(
                    stores.probe_source(&config),
                    CredentialSource::UserConfigPlaintext
                );
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    /// The token comes from the system config only: the merged config has it,
    /// but the user-file probe (`PlaintextStore::get`) does not. Modeled on
    /// `set_by_system_config` (config/mod.rs:581).
    #[test]
    fn probe_returns_system_config_when_token_only_from_system() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();
        let system_dir = tempfile::tempdir().unwrap();
        write_flox_toml(user_dir.path(), "");
        write_flox_toml(system_dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));

        temp_env::with_vars(
            probe_vars(home.path(), user_dir.path(), system_dir.path(), None),
            || {
                let config = Config::parse().unwrap();
                // Sanity: the token reached the merged config from /etc only.
                assert_eq!(config.flox.floxhub_token.as_deref(), Some(TOKEN));

                let plaintext =
                    CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                assert_eq!(plaintext.get().unwrap(), None);
                let keyring = CredentialStoreImpl::Mock(MockStore::new());
                let stores = CredentialStores::from_stores(keyring, plaintext);
                assert_eq!(stores.probe_source(&config), CredentialSource::SystemConfig);
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    /// The keyring is consulted last: with env and both config files empty, a
    /// token in the (mocked) keyring resolves to `Keyring`. Models the
    /// `status reports keyring source` validation item.
    #[test]
    fn probe_returns_keyring_when_only_keyring_has_token() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();
        let system_dir = tempfile::tempdir().unwrap();
        write_flox_toml(user_dir.path(), "");
        write_flox_toml(system_dir.path(), "");

        temp_env::with_vars(
            probe_vars(home.path(), user_dir.path(), system_dir.path(), None),
            || {
                let config = Config::parse().unwrap();
                let plaintext =
                    CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                let keyring = CredentialStoreImpl::Mock(MockStore::new());
                keyring.set(TOKEN).unwrap();
                let stores = CredentialStores::from_stores(keyring, plaintext);
                assert_eq!(stores.probe_source(&config), CredentialSource::Keyring);
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    /// Reproduce the real `status` call order: the resolver populates
    /// `config.flox.floxhub_token` from the keyring, then `probe` runs on that
    /// mutated config. The keyring branch must win over the `SystemConfig`
    /// inference — otherwise a keyring-sourced token misreports as system
    /// config. (`Config::parse()` is unnecessary here: the resolver works on
    /// the merged field directly, which is what the bug hinges on.)
    #[test]
    fn probe_after_resolver_reports_keyring_not_system_config() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set(TOKEN).unwrap();
        let plaintext = CredentialStoreImpl::Mock(MockStore::new());
        let stores = CredentialStores::from_stores(keyring, plaintext);

        let mut config = config_with_token(None);
        stores.resolve_into(&mut config);

        assert_eq!(stores.probe_source(&config), CredentialSource::Keyring);
    }

    /// `/etc/flox.toml` supplies the merged token while the keyring holds a
    /// *different* credential: the probe must report `SystemConfig`, not
    /// `Keyring`, so an invalid system token never routes `clear_invalid` to
    /// the user's unrelated saved keyring credential.
    #[test]
    fn probe_reports_system_config_when_keyring_holds_different_token() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            keyring.set("keyring-token").unwrap();
            let plaintext = CredentialStoreImpl::Mock(MockStore::new());
            let stores = CredentialStores::from_stores(keyring.clone(), plaintext);

            // Mirror the merge: the system config supplied the (invalid) token,
            // and the resolver leaves a non-empty merged token untouched.
            let mut config = config_with_token(Some("invalid-system-token"));
            assert_eq!(stores.resolve_into(&mut config), ResolveOutcome::Unchanged);

            let source = stores.probe_source(&config);
            assert_eq!(source, CredentialSource::SystemConfig);

            // The invalid-token cleanup routed by this source must preserve
            // the keyring credential.
            stores.clear_invalid(source);
            assert_eq!(keyring.get().unwrap(), Some("keyring-token".to_string()));
        });
    }

    // --- CredentialStores::persist_login_token: the login storage decision ---

    /// Default login: the token goes to the keyring and no plaintext token is
    /// left behind.
    #[test]
    fn login_stores_in_keyring_and_clears_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        // A pre-existing plaintext token must be removed once the keyring write
        // confirms, so it cannot shadow the keyring entry on the next read.
        write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));

        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

        let storage = stores
            .persist_login_token(TOKEN, TokenStorageMode::Keyring)
            .unwrap();

        assert_eq!(storage, TokenStorage::Keyring);
        assert_eq!(keyring.get().unwrap(), Some(TOKEN.to_string()));
        assert_eq!(plaintext.get().unwrap(), None);
    }

    /// On any keyring failure, login falls back to plaintext and signals it so
    /// the caller can warn.
    #[test]
    fn login_falls_back_to_plaintext_on_keyring_error() {
        let dir = tempfile::tempdir().unwrap();
        let keyring_mock = MockStore::new();
        keyring_mock.set_error("no backend");
        let keyring = CredentialStoreImpl::Mock(keyring_mock);
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

        let storage = stores
            .persist_login_token(TOKEN, TokenStorageMode::Keyring)
            .unwrap();

        assert_eq!(storage, TokenStorage::Plaintext);
        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }

    /// A keyring write succeeds but the plaintext cleanup fails (e.g. the
    /// config file is unreadable). Login must still succeed — the token is
    /// already safely stored in the keyring — rather than turning a best-effort
    /// cleanup step into a hard login failure.
    #[test]
    fn login_succeeds_when_keyring_stored_but_plaintext_cleanup_fails() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        let plaintext_mock = MockStore::new();
        plaintext_mock.set_error("config file is unreadable");
        let plaintext = CredentialStoreImpl::Mock(plaintext_mock);
        let stores = CredentialStores::from_stores(keyring.clone(), plaintext);

        let storage = stores
            .persist_login_token(TOKEN, TokenStorageMode::Keyring)
            .unwrap();

        assert_eq!(storage, TokenStorage::Keyring);
        assert_eq!(keyring.get().unwrap(), Some(TOKEN.to_string()));
    }

    /// A `Plaintext` target forces the plaintext file even when the keyring
    /// write would have succeeded; the keyring is never written.
    #[test]
    fn login_plaintext_target_forces_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

        let storage = stores
            .persist_login_token(TOKEN, TokenStorageMode::Plaintext)
            .unwrap();

        assert_eq!(storage, TokenStorage::Plaintext);
        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }

    /// Storing plain text drops any pre-existing keyring entry so it is not left
    /// behind as a stale secret (e.g. to resurface on a later read if the
    /// plain-text file is removed).
    #[test]
    fn login_plaintext_target_removes_stale_keyring_entry() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set("stale-keyring-token").unwrap();
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

        let storage = stores
            .persist_login_token(TOKEN, TokenStorageMode::Plaintext)
            .unwrap();

        assert_eq!(storage, TokenStorage::Plaintext);
        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }

    // --- CredentialStores::resolve_into: the upstream read resolver ---

    fn config_with_token(token: Option<&str>) -> Config {
        let mut config = Config::default();
        config.flox.floxhub_token = token.map(str::to_string);
        config
    }

    /// When the merged config supplied no token, the keyring value populates it.
    #[test]
    fn resolve_populates_token_from_keyring_when_config_empty() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            keyring.set(TOKEN).unwrap();
            // Empty plaintext store: nothing to migrate, so the read path runs.
            let plaintext = CredentialStoreImpl::Mock(MockStore::new());
            let stores = CredentialStores::from_stores(keyring, plaintext);

            let mut config = config_with_token(None);
            let outcome = stores.resolve_into(&mut config);

            assert_eq!(outcome, ResolveOutcome::PopulatedFromKeyring);
            assert_eq!(config.flox.floxhub_token.as_deref(), Some(TOKEN));
        });
    }

    /// A non-empty merged token wins: env > user file > system all flow through this
    /// field, so the keyring is not consulted and the value is untouched.
    #[test]
    fn resolve_leaves_existing_token_untouched() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            keyring.set("keyring-token").unwrap();
            // Empty plaintext store: no migration, so only the read path could
            // touch the config — and it must not, because the token is set.
            let plaintext = CredentialStoreImpl::Mock(MockStore::new());
            let stores = CredentialStores::from_stores(keyring, plaintext);

            let mut config = config_with_token(Some("config-token"));
            let outcome = stores.resolve_into(&mut config);

            assert_eq!(outcome, ResolveOutcome::Unchanged);
            assert_eq!(config.flox.floxhub_token.as_deref(), Some("config-token"));
        });
    }

    // --- CredentialStores::resolve_into: opportunistic plaintext → keyring migration ---

    /// A user-file plaintext token is moved into the keyring and removed from
    /// the file once the keyring write confirms.
    #[test]
    fn resolve_migrates_plaintext_token_to_keyring() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let dir = tempfile::tempdir().unwrap();
            write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
            let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
            let keyring = CredentialStoreImpl::Mock(MockStore::new());

            let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

            // Mirror the merge: the user-file token is already in the config.
            let mut config = config_with_token(Some(TOKEN));
            let outcome = stores.resolve_into(&mut config);

            assert_eq!(outcome, ResolveOutcome::Migrated);
            assert_eq!(keyring.get().unwrap(), Some(TOKEN.to_string()));
            assert_eq!(plaintext.get().unwrap(), None);
            // Migration is store-to-store only: the config field is left as the
            // merge produced it.
            assert_eq!(config.flox.floxhub_token.as_deref(), Some(TOKEN));
        });
    }

    /// When the standing storage preference is plain text, a user-file token is
    /// not migrated into the keyring: the keyring is never written and the
    /// plain-text token stays on disk.
    #[test]
    fn resolve_skips_migration_when_storage_is_plaintext() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let dir = tempfile::tempdir().unwrap();
            write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
            let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

            let mut config = config_with_token(Some(TOKEN));
            config.flox.floxhub_token_storage = TokenStorageMode::Plaintext;
            let outcome = stores.resolve_into(&mut config);

            assert_eq!(outcome, ResolveOutcome::Unchanged);
            assert_eq!(keyring.get().unwrap(), None);
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
        });
    }

    /// `FLOX_FLOXHUB_TOKEN` set → the plaintext token is not migrated (the env
    /// token is transient and must not be persisted to the keyring).
    #[test]
    fn resolve_does_not_migrate_when_env_token_set() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, Some("env-token"), || {
            let dir = tempfile::tempdir().unwrap();
            write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
            let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
            let keyring = CredentialStoreImpl::Mock(MockStore::new());

            let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

            let mut config = config_with_token(Some("env-token"));
            let outcome = stores.resolve_into(&mut config);

            assert_eq!(outcome, ResolveOutcome::Unchanged);
            assert_eq!(keyring.get().unwrap(), None);
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
        });
    }

    /// An explicit *empty* `FLOX_FLOXHUB_TOKEN` export masks saved credentials
    /// for one invocation: the resolver must neither migrate the plaintext
    /// token nor populate the config from the keyring, so the invocation stays
    /// logged out.
    #[test]
    fn resolve_is_inert_when_env_token_is_empty() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, Some(""), || {
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            keyring.set("keyring-token").unwrap();
            let plaintext = CredentialStoreImpl::Mock(MockStore::new());
            plaintext.set(TOKEN).unwrap();
            let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

            // Mirror the merge: the empty env override yields an empty merged
            // token.
            let mut config = config_with_token(Some(""));
            let outcome = stores.resolve_into(&mut config);

            assert_eq!(outcome, ResolveOutcome::Unchanged);
            // No migration: both stores are exactly as they were.
            assert_eq!(keyring.get().unwrap(), Some("keyring-token".to_string()));
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
            // No populate: the masked (empty) token is left in place.
            assert_eq!(config.flox.floxhub_token.as_deref(), Some(""));
        });
    }

    /// Keyring write fails → the plaintext file is left untouched (no data
    /// loss, no migration).
    #[test]
    fn resolve_leaves_plaintext_untouched_when_keyring_write_fails() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let dir = tempfile::tempdir().unwrap();
            write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
            let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
            let keyring_mock = MockStore::new();
            // The injected error lands on the migration's `set` call — the
            // migration branch never calls `keyring.get()` first.
            keyring_mock.set_error("no backend");
            let keyring = CredentialStoreImpl::Mock(keyring_mock);
            let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

            let mut config = config_with_token(Some(TOKEN));
            let outcome = stores.resolve_into(&mut config);

            assert_eq!(outcome, ResolveOutcome::Unchanged);
            assert_eq!(keyring.get().unwrap(), None);
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
        });
    }

    /// Keyring write succeeds but the plaintext removal fails (e.g. an unwritable
    /// `flox.toml`). The token is now in the keyring, but the plaintext copy
    /// lingers, so the resolver reports `MigratedButPlaintextRemains` (not a
    /// silent `Unchanged`) and the caller warns instead of looping silently.
    #[test]
    fn resolve_reports_plaintext_remains_when_remove_fails_after_keyring_write() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let plaintext_mock = MockStore::new();
            plaintext_mock.set(TOKEN).unwrap();
            plaintext_mock.set_remove_error("flox.toml is not writable");
            let plaintext = CredentialStoreImpl::Mock(plaintext_mock);
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

            let mut config = config_with_token(Some(TOKEN));
            let outcome = stores.resolve_into(&mut config);

            assert_eq!(outcome, ResolveOutcome::MigratedButPlaintextRemains);
            // The keyring received the token; the plaintext copy still lingers.
            assert_eq!(keyring.get().unwrap(), Some(TOKEN.to_string()));
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
        });
    }

    // --- CredentialStores::clear_invalid: invalid-token cleanup routing ---

    /// A keyring-sourced invalid token is cleared from the keyring only; a
    /// plaintext credential that did not supply it is left intact.
    #[test]
    fn clear_invalid_credential_removes_only_keyring_for_keyring_source() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set(TOKEN).unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_flox_toml(dir.path(), "floxhub_token = \"other-token\"\n");
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

        stores.clear_invalid(CredentialSource::Keyring);

        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some("other-token".to_string()));
    }

    /// A plaintext-file-sourced invalid token is cleared from the file only; a
    /// keyring credential that did not supply it is left intact.
    #[test]
    fn clear_invalid_credential_removes_only_plaintext_for_user_file_source() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set("other-token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

        stores.clear_invalid(CredentialSource::UserConfigPlaintext);

        assert_eq!(plaintext.get().unwrap(), None);
        assert_eq!(keyring.get().unwrap(), Some("other-token".to_string()));
    }

    /// An invalid token from `FLOX_FLOXHUB_TOKEN` (or system config) must not
    /// delete the user's saved keyring/plaintext credential — those did not
    /// supply the bad token, and clearing them would force a needless re-login
    /// once the env/system value is corrected.
    #[test]
    fn clear_invalid_credential_preserves_saved_stores_for_env_source() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set(TOKEN).unwrap();
        let dir = tempfile::tempdir().unwrap();
        write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let stores = CredentialStores::from_stores(keyring.clone(), plaintext.clone());

        stores.clear_invalid(CredentialSource::Env);

        assert_eq!(keyring.get().unwrap(), Some(TOKEN.to_string()));
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }

    // --- CredentialStores::remove_all: logout removal ---

    /// A keyring platform failure (e.g. a locked Secret Service session) must
    /// not short-circuit logout: the plaintext token is still removed, and the
    /// keyring error is still surfaced to the caller.
    #[test]
    fn remove_all_clears_plaintext_even_when_keyring_remove_fails() {
        let keyring_mock = MockStore::new();
        keyring_mock.set_error("keyring is locked");
        let keyring = CredentialStoreImpl::Mock(keyring_mock);
        let plaintext = CredentialStoreImpl::Mock(MockStore::new());
        plaintext.set(TOKEN).unwrap();
        let stores = CredentialStores::from_stores(keyring, plaintext.clone());

        let result = stores.remove_all();

        assert!(result.is_err());
        assert_eq!(plaintext.get().unwrap(), None);
    }
}
