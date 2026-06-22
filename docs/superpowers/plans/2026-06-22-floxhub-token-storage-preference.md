# FloxHub Token-Storage Preference Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `flox auth login --insecure-storage` a *persistent* token-storage preference (`floxhub_token_storage = "keyring" | "plaintext"`), add a temporary `--insecure-storage --once`, and gate plaintext→keyring migration on the preference.

**Architecture:** A new two-valued config enum `TokenStorageMode` becomes the single source of truth for *where tokens go*. It is consulted at login (where to write) and at migration (whether to move an existing plaintext token). `--insecure-storage` writes the preference (unless `--once`); a plain login honors whatever standing preference exists. The migration in `resolve_credential_into` runs only when the preference is `Keyring`.

**Tech Stack:** Rust 2024, `bpaf` (CLI parsing), `serde`/`toml_edit` (config), `indoc`/`formatdoc` (messages), existing `CredentialStore`/`MockStore` test harness.

## Global Constraints

- **Build/test wrapping:** `IN_NIX_SHELL` is unset in this environment — every `cargo`/`just`/`git push` command MUST be wrapped with `nix develop -c`. (If `IN_NIX_SHELL` is set, run directly.)
- **Already done in the base branch (do NOT reintroduce):** Folded-in fixes #1 (`ResolveOutcome::MigratedButPlaintextRemains` + the call-site warning in `commands/mod.rs:313`) and #3 (`indoc!` in `persist_login_token`) are **already implemented** on `implement-secure-cli-credential-storage-with-keyring-fallback`. The ONLY migration-block change in this plan is prepending the `storage == TokenStorageMode::Keyring &&` gate. Do not paste the spec's illustrative fix-#1 block — it is an older formulation that returns `Unchanged` and would undo committed work.
- **Enum shape:** `TokenStorageMode` mirrors the existing `AutoActivate` enum exactly, plus `Copy`: `#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]`, `#[serde(rename_all = "lowercase")]`, with `#[default] Keyring`.
- **Config commands need no changes:** `flox config --set/--delete floxhub_token_storage` works generically — `Config::write_to` re-parses the whole document into `Config`, validating the value against the enum for free.
- **Message conventions (AGENTS.md):** complete sentences, sentence case, suggest next steps, single-quote suggested commands (`'flox config --delete floxhub_token_storage'`), one emoji max per response. Prefer overlong lines to `\`-continued messages.
- **Test naming:** no `test_` prefix; name for the behavior verified. Use `assert_eq!` on whole values. `pretty_assertions::assert_eq` is already imported in the `credential_store.rs` test module.
- **Branch:** `floxhub-token-storage-preference`, stacked on `implement-secure-cli-credential-storage-with-keyring-fallback`. PR base = the keyring branch (retarget to `main` after #4420 merges).

---

## File Structure

| File | Responsibility | Change |
|------|----------------|--------|
| `cli/flox/src/config/mod.rs` | Define `TokenStorageMode`; add `floxhub_token_storage` field to `FloxConfig`; parse test. | Modify |
| `cli/flox/src/utils/credential_store.rs` | `resolve_credential_into` migration gate; `persist_login_token` target-mode + stale-keyring removal; unit tests. | Modify |
| `cli/flox/src/commands/auth.rs` | `--once` flag; `login_flox` target/persist logic; `auth status` plaintext line. | Modify |
| `cli/flox/src/commands/mod.rs` | Pass `storage` to `resolve_credential_into`; pass preference to the two `login_flox` callers. | Modify |

---

## Task 1: Add the `TokenStorageMode` config field

**Files:**
- Modify: `cli/flox/src/config/mod.rs` (enum after `AutoActivate` ~line 190; field in `FloxConfig` after `floxhub_authn_mode` ~line 89)
- Test: `cli/flox/src/config/mod.rs` (the `#[cfg(test)] mod tests` block)

**Interfaces:**
- Produces: `pub enum TokenStorageMode { Keyring, Plaintext }` (default `Keyring`, `Copy`), and `FloxConfig.floxhub_token_storage: TokenStorageMode` (TOML key `floxhub_token_storage`).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `cli/flox/src/config/mod.rs` (it already has `use super::*;`, `fs`, `tempfile`, and `mock_flox_dirs()`):

```rust
    #[test]
    fn floxhub_token_storage_parses_and_defaults() {
        let user_config_dir = tempfile::tempdir().unwrap();
        let system_config_dir = tempfile::tempdir().unwrap();
        fs::write(system_config_dir.path().join(FLOX_CONFIG_FILE), "").unwrap();

        // Absent → defaults to keyring.
        fs::write(user_config_dir.path().join(FLOX_CONFIG_FILE), "").unwrap();
        let config = Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            [],
        )
        .unwrap();
        assert_eq!(config.flox.floxhub_token_storage, TokenStorageMode::Keyring);

        // Explicit plaintext → parsed.
        fs::write(
            user_config_dir.path().join(FLOX_CONFIG_FILE),
            "floxhub_token_storage = \"plaintext\"\n",
        )
        .unwrap();
        let config = Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            [],
        )
        .unwrap();
        assert_eq!(config.flox.floxhub_token_storage, TokenStorageMode::Plaintext);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c cargo test -p flox --lib config::tests::floxhub_token_storage_parses_and_defaults`
Expected: **compile error** — no field `floxhub_token_storage` on `FloxConfig` and no `TokenStorageMode`. (A test that does not compile is the "fail" state in Rust.)

- [ ] **Step 3: Add the enum**

In `cli/flox/src/config/mod.rs`, after the `AutoActivate` enum (immediately after its closing `}` near line 190), add:

```rust
/// Where `flox auth login` stores the FloxHub token.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TokenStorageMode {
    /// Store the token in the OS-native keyring (default).
    #[default]
    Keyring,
    /// Store the token in plain text in flox.toml.
    Plaintext,
}
```

- [ ] **Step 4: Add the field**

In `FloxConfig`, immediately after the `floxhub_authn_mode` field (line 89), add:

```rust
    /// Where new FloxHub tokens are stored: the OS keyring (default) or plain
    /// text in flox.toml. Set to `plaintext` by
    /// `flox auth login --insecure-storage`; cleared with
    /// `flox config --delete floxhub_token_storage`.
    #[serde(default)]
    pub floxhub_token_storage: TokenStorageMode,
```

- [ ] **Step 5: Run test to verify it passes**

Run: `nix develop -c cargo test -p flox --lib config::tests::floxhub_token_storage_parses_and_defaults`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
nix develop -c git add cli/flox/src/config/mod.rs
nix develop -c git commit -m "feat(auth): add floxhub_token_storage config preference"
```

---

## Task 2: Gate plaintext→keyring migration on the storage preference

**Files:**
- Modify: `cli/flox/src/utils/credential_store.rs` (import ~line 28; `resolve_credential_into` signature ~line 565 and migration `if` ~line 585; all in-module test call sites)
- Modify: `cli/flox/src/commands/mod.rs` (import line 80; call site ~line 302)
- Test: `cli/flox/src/utils/credential_store.rs` (tests module)

**Interfaces:**
- Consumes: `TokenStorageMode` (Task 1).
- Produces: `resolve_credential_into(config, keyring, plaintext, is_hook, is_auth0, storage: TokenStorageMode) -> ResolveOutcome`. Migration runs only when `storage == TokenStorageMode::Keyring`.

- [ ] **Step 1: Write the failing test**

Add to the migration test group in `cli/flox/src/utils/credential_store.rs` (after `resolve_does_not_migrate_outside_auth0_mode`, ~line 1172):

```rust
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

            let mut config = config_with_token(Some(TOKEN));
            let outcome = resolve_credential_into(
                &mut config,
                &keyring,
                &plaintext,
                false,
                true,
                TokenStorageMode::Plaintext,
            );

            assert_eq!(outcome, ResolveOutcome::Unchanged);
            assert_eq!(keyring.get().unwrap(), None);
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
        });
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c cargo test -p flox --lib credential_store`
Expected: **compile error** — `resolve_credential_into` takes 5 args, not 6, and `TokenStorageMode` is unresolved in this module.

- [ ] **Step 3: Add the import**

In `cli/flox/src/utils/credential_store.rs`, change line 28:

```rust
use crate::config::{Config, FLOX_CONFIG_FILE};
```
to:
```rust
use crate::config::{Config, FLOX_CONFIG_FILE, TokenStorageMode};
```

- [ ] **Step 4: Add the parameter and the gate**

Change the `resolve_credential_into` signature (~line 565) to add a final parameter:

```rust
pub fn resolve_credential_into(
    config: &mut Config,
    keyring: &CredentialStoreImpl,
    plaintext: &CredentialStoreImpl,
    is_hook: bool,
    is_auth0: bool,
    storage: TokenStorageMode,
) -> ResolveOutcome {
```

Then change ONLY the migration `if` condition (~line 585) — leave the `keyring.set` / `plaintext.remove` / `MigratedButPlaintextRemains` body exactly as-is:

```rust
    if storage == TokenStorageMode::Keyring && !env_set && let Ok(Some(token)) = plaintext.get() {
```

Update the doc comment's migration-conditions list (~line 544) to add a bullet:

```rust
/// - the storage preference is `Keyring` (`storage`) — when the user has chosen
///   plain-text storage, a plaintext token is left in place rather than moved.
```

- [ ] **Step 5: Update the production call site**

In `cli/flox/src/commands/mod.rs`, add `TokenStorageMode` to the config import (line 80):

```rust
use crate::config::{Config, EnvironmentTrust, FLOX_DIR_NAME, TokenStorageMode};
```

Then change the call site (~line 301-308). Read `storage` into a local **before** the call to avoid borrowing `config` while it is mutably borrowed (`TokenStorageMode` is `Copy`, mirroring the `is_auth0` line above it):

```rust
        let is_auth0 = matches!(config.flox.floxhub_authn_mode, AuthnMode::Auth0);
        let storage = config.flox.floxhub_token_storage;
        let outcome = resolve_credential_into(
            &mut config,
            &keyring,
            &plaintext,
            self.is_prompt_hook_flow(),
            is_auth0,
            storage,
        );
```

- [ ] **Step 6: Update existing in-module test call sites**

In `cli/flox/src/utils/credential_store.rs`, every existing `resolve_credential_into(&mut config, &keyring, &plaintext, <is_hook>, <is_auth0>)` call must gain a trailing `, TokenStorageMode::Keyring` (the default preference, correct for all existing migration/read tests). These are at lines ~968, 1064, 1083, 1099, 1119, 1141, 1164, 1189, 1211, 1229. Example — line 1064:

```rust
            let outcome = resolve_credential_into(
                &mut config,
                &keyring,
                &plaintext,
                false,
                true,
                TokenStorageMode::Keyring,
            );
```

Apply the same `, TokenStorageMode::Keyring` to each call. (The line-968 call inside `probe_after_resolver_reports_keyring_not_system_config` discards the result; add the arg there too.)

- [ ] **Step 7: Run tests to verify they pass**

Run: `nix develop -c cargo test -p flox --lib credential_store`
Expected: PASS, including `resolve_skips_migration_when_storage_is_plaintext` and all pre-existing `resolve_*` tests.

- [ ] **Step 8: Commit**

```bash
nix develop -c git add cli/flox/src/utils/credential_store.rs cli/flox/src/commands/mod.rs
nix develop -c git commit -m "feat(auth): gate token migration on floxhub_token_storage preference"
```

---

## Task 3: Login honors the preference (`persist_login_token` target + `--once`)

**Files:**
- Modify: `cli/flox/src/utils/credential_store.rs` (`persist_login_token` ~line 492; its 4 tests ~lines 980-1044)
- Modify: `cli/flox/src/commands/auth.rs` (imports; `Auth::Login` arm ~line 267; `login_flox` ~line 364)
- Modify: `cli/flox/src/commands/mod.rs` (`ensure_auth` call ~line 1505)
- Test: `cli/flox/src/utils/credential_store.rs` (persist test group)

**Interfaces:**
- Consumes: `TokenStorageMode` (Task 1); `update_config` (`crate::commands::general::update_config`).
- Produces: `persist_login_token(token, target: TokenStorageMode, keyring, plaintext) -> Result<TokenStorage, CredentialStoreError>`; `login_flox(flox, insecure_storage: bool, once: bool, storage_pref: TokenStorageMode) -> Result<String>`.

- [ ] **Step 1: Write the failing test**

Add to the persist test group in `cli/flox/src/utils/credential_store.rs` (after `login_insecure_storage_forces_plaintext`, ~line 1044):

```rust
    /// Storing plain text drops any pre-existing keyring entry so it cannot
    /// resurface on the next read and shadow the user's plain-text choice.
    #[test]
    fn login_plaintext_target_removes_stale_keyring_entry() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set("stale-keyring-token").unwrap();
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));

        let storage =
            persist_login_token(TOKEN, TokenStorageMode::Plaintext, &keyring, &plaintext).unwrap();

        assert_eq!(storage, TokenStorage::Plaintext);
        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `nix develop -c cargo test -p flox --lib credential_store::tests::login_plaintext_target_removes_stale_keyring_entry`
Expected: **compile error** — `persist_login_token` takes a `bool`, not `TokenStorageMode`.

- [ ] **Step 3: Change `persist_login_token` to take a target mode**

Replace the `persist_login_token` body (~lines 492-514) with:

```rust
pub fn persist_login_token(
    token: &str,
    target: TokenStorageMode,
    keyring: &CredentialStoreImpl,
    plaintext: &CredentialStoreImpl,
) -> Result<TokenStorage, CredentialStoreError> {
    if target == TokenStorageMode::Keyring && keyring.set(token).is_ok() {
        // The keyring already holds the token, so a failure to remove the old
        // plaintext copy must not fail the login. Warn instead: a lingering
        // plaintext token both leaves a secret on disk and shadows the keyring
        // on the next read (user file > keyring).
        if let Err(e) = plaintext.remove() {
            tracing::warn!("could not remove the plaintext credential after a keyring write: {e}");
            message::warning(indoc! {"
                Stored your credential in the system keyring, but could not remove the existing plain-text credential from flox.toml.
                Remove the 'floxhub_token' line from flox.toml so it does not shadow the keyring."});
        }
        return Ok(TokenStorage::Keyring);
    }

    plaintext.set(token)?;
    // An explicit plain-text choice supersedes any keyring entry: drop a
    // lingering keyring token (best effort) so a later read cannot surface it
    // and shadow the plain-text file the user just chose. Scoped to the explicit
    // `Plaintext` target — on a keyring-write fallback there is nothing of ours
    // in the keyring to remove.
    if target == TokenStorageMode::Plaintext {
        if let Err(e) = keyring.remove() {
            tracing::debug!("could not remove the keyring credential after storing plain text: {e}");
        }
    }
    Ok(TokenStorage::Plaintext)
}
```

Update its doc comment (~lines 485-491) to describe the target parameter instead of `insecure_storage`:

```rust
/// Persist a logged-in token according to `target`.
///
/// `Keyring`: attempt the keyring first (try-then-confirm); on success store
/// there and remove any lingering plaintext token so it cannot shadow the
/// keyring entry, and on any keyring failure fall back to the plaintext file
/// (`0600`). `Plaintext`: write the plaintext file and drop any existing keyring
/// entry (best effort). The returned [TokenStorage] tells the caller whether to
/// warn the user.
```

- [ ] **Step 4: Update the three existing keyring-path persist tests**

In `cli/flox/src/utils/credential_store.rs`, change the `false` argument to `TokenStorageMode::Keyring` in these three tests:
- `login_stores_in_keyring_and_clears_plaintext` (~line 990)
- `login_falls_back_to_plaintext_on_keyring_error` (~line 1007)
- `login_succeeds_when_keyring_stored_but_plaintext_cleanup_fails` (~line 1025)

Each becomes, e.g.:

```rust
        let storage =
            persist_login_token(TOKEN, TokenStorageMode::Keyring, &keyring, &plaintext).unwrap();
```

- [ ] **Step 5: Rename and update the forced-plaintext test**

Replace `login_insecure_storage_forces_plaintext` (~lines 1031-1044) with the renamed, target-based version:

```rust
    /// A `Plaintext` target forces the plaintext file even when the keyring
    /// write would have succeeded; the keyring is never written.
    #[test]
    fn login_plaintext_target_forces_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));

        let storage =
            persist_login_token(TOKEN, TokenStorageMode::Plaintext, &keyring, &plaintext).unwrap();

        assert_eq!(storage, TokenStorage::Plaintext);
        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }
```

- [ ] **Step 6: Add the `--once` flag and thread the preference through `login_flox`**

In `cli/flox/src/commands/auth.rs`:

Add imports — extend the `crate::config` import (line 34) and add the `update_config` import below it:

```rust
use crate::config::{Config, FLOX_CONFIG_FILE, TokenStorageMode};
use crate::commands::general::update_config;
```

Add the `once` flag to the `Login` variant (~lines 242-246):

```rust
    Login {
        /// Store the token in plain text in flox.toml instead of the OS keyring
        #[bpaf(long("insecure-storage"))]
        insecure_storage: bool,
        /// With --insecure-storage, store plain text only for this login without
        /// changing the saved storage preference
        #[bpaf(long("once"))]
        once: bool,
    },
```

Update the `Auth::Login` match arm (~lines 267-272):

```rust
            Auth::Login {
                insecure_storage,
                once,
            } => {
                let span = tracing::info_span!("login");
                let _guard = span.enter();
                login_flox(
                    &mut flox,
                    insecure_storage,
                    once,
                    config.flox.floxhub_token_storage,
                )
                .await?;
                Ok(())
            },
```

Replace the `login_flox` signature and storage logic (~lines 364-397). Keep the OAuth/`authorize` lines unchanged; change from the `insecure_storage` argument onward:

```rust
pub async fn login_flox(
    flox: &mut Flox,
    insecure_storage: bool,
    once: bool,
    storage_pref: TokenStorageMode,
) -> Result<String> {
    let client = create_oauth_client()?;
    let cred = authorize(client, flox.floxhub.base_url())
        .await
        .context("Could not authorize via oauth")?;

    debug!("Credentials received: {cred:#?}");
    debug!("Writing token to config");

    // set the token in the runtime config
    let token = FloxhubToken::new(cred.token)?;
    let handle = token.handle().to_string();

    // `--insecure-storage` forces plain text for this login; otherwise honor the
    // standing storage preference.
    let target = if insecure_storage {
        TokenStorageMode::Plaintext
    } else {
        storage_pref
    };

    // Persist the plain-text choice as a standing preference only when
    // `--insecure-storage` is given without `--once`. `--once` stores plain text
    // this one time without changing where future tokens go, so the token is
    // re-secured to the keyring once the keyring is available again.
    if insecure_storage && !once {
        update_config(
            &flox.config_dir,
            "floxhub_token_storage",
            Some(TokenStorageMode::Plaintext),
        )
        .context("Could not save the token-storage preference")?;
    }

    let keyring = CredentialStoreImpl::Keyring(KeyringStore::new(flox.floxhub.base_url()));
    let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(&flox.config_dir));
    let storage = persist_login_token(token.secret(), target, &keyring, &plaintext)
        .context("Could not store token")?;

    let auth_context = AuthContext::from_mode(&AuthnMode::Auth0, Some(token.clone()));
    let _ = flox.set_auth_context(auth_context);

    message::updated("Authentication complete");
    message::updated(format!("Logged in as {handle}"));

    if storage == TokenStorage::Plaintext {
        message::warning(formatdoc! {"
            Credential stored in plain text at '{}'.
            No OS keyring is available, or plain-text storage was requested.",
            flox.config_dir.join(FLOX_CONFIG_FILE).display()
        });
    }

    Ok(handle)
}
```

- [ ] **Step 7: Update the implicit re-auth caller**

In `cli/flox/src/commands/mod.rs`, update the `ensure_auth` call (~lines 1504-1505). `TokenStorageMode` is already imported (Task 2, Step 5):

```rust
            // Implicit re-authentication stores to the secure default (keyring).
            // The standing storage preference is not threaded through the many
            // `ensure_auth` call sites, so an implicit re-login does not honor a
            // `plaintext` preference; an explicit `flox auth login` does.
            auth::login_flox(flox, false, false, TokenStorageMode::Keyring).await
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `nix develop -c cargo test -p flox --lib credential_store`
Expected: PASS — `login_plaintext_target_removes_stale_keyring_entry`, `login_plaintext_target_forces_plaintext`, and the three keyring-path tests.

Then build the whole crate to confirm the `auth.rs`/`mod.rs` wiring compiles:

Run: `nix develop -c cargo build -p flox`
Expected: builds clean.

- [ ] **Step 9: Commit**

```bash
nix develop -c git add cli/flox/src/utils/credential_store.rs cli/flox/src/commands/auth.rs cli/flox/src/commands/mod.rs
nix develop -c git commit -m "feat(auth): persist --insecure-storage preference, add --once"
```

---

## Task 4: `flox auth status` reports a plain-text preference

**Files:**
- Modify: `cli/flox/src/commands/auth.rs` (`Auth::Status` arm, after the source-reporting `match`, ~line 336)

**Interfaces:**
- Consumes: `TokenStorageMode` (imported in Task 3), `config.flox.floxhub_token_storage`.

- [ ] **Step 1: Add the preference line**

In the `Auth::Status` arm, immediately after the `match source { … }` block and before `Ok(())` (~line 336), add:

```rust
                if config.flox.floxhub_token_storage == TokenStorageMode::Plaintext {
                    message::plain(formatdoc! {"
                        Token storage preference is set to plain text.
                        Run 'flox config --delete floxhub_token_storage' to store tokens in the system keyring."});
                }
```

- [ ] **Step 2: Verify it compiles**

Run: `nix develop -c cargo build -p flox`
Expected: builds clean (`formatdoc` and `config` are already in scope in `auth.rs`).

- [ ] **Step 3: Manual smoke (optional, documents the behavior)**

```bash
nix develop -c cargo run -p flox -- config --set floxhub_token_storage plaintext
nix develop -c cargo run -p flox -- config --delete floxhub_token_storage
```
Expected: both succeed; `--set garbage` would be rejected by config validation.

- [ ] **Step 4: Commit**

```bash
nix develop -c git add cli/flox/src/commands/auth.rs
nix develop -c git commit -m "feat(auth): note plain-text storage preference in auth status"
```

---

## Final Verification

- [ ] **Full crate test suite** (the `--once` flag changes `flox auth login --help`; a usage/bats snapshot may assert on it):

Run: `nix develop -c cargo test -p flox`
Expected: PASS.

- [ ] **Lint:**

Run: `nix develop -c cargo clippy --all`
Expected: no warnings.

- [ ] **Format:**

Run: `nix develop -c cargo fmt --all` then `nix develop -c git diff --exit-code`
Expected: no diff (already formatted).

---

## Spec Coverage Check

| Spec requirement | Task |
|---|---|
| `TokenStorageMode` enum + `floxhub_token_storage` field, lowercase serde, default keyring | Task 1 |
| Parses from `flox.toml`; defaults to keyring when absent | Task 1 (test) |
| Migration runs only when preference is `Keyring`; read-fallback unchanged | Task 2 |
| `resolve_credential_into` gains `storage` param computed at call site | Task 2 |
| Fix #1 (`MigratedButPlaintextRemains`) preserved, not reverted | Constraint + Task 2 (gate only) |
| Fix #3 (`indoc!`) preserved | Already in base; `persist_login_token` keeps `indoc!` (Task 3) |
| `--once` flag on `Login` | Task 3 |
| `login_flox`: compute target, persist pref only when `insecure && !once` | Task 3 |
| Plaintext target stores plaintext + removes stale keyring entry | Task 3 |
| Keyring target uses keyring-first-with-fallback | Task 3 |
| Plain login / implicit re-auth honor a standing preference (explicit; re-auth keeps secure default by design) | Task 3 |
| `flox config --set/--delete floxhub_token_storage` | Constraint (works generically; verified Task 4) |
| `auth status` reports plain-text preference + revert command | Task 4 |
| `--secure-storage`, per-env/time-boxed storage, runtime-rep changes — OUT | Not implemented (YAGNI) |
