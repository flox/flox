# FloxHub token-storage preference (persistent vs. temporary `--insecure-storage`)

**Date:** 2026-06-22
**Status:** Approved design — ready for implementation plan
**Related:** PR #4420 (secure CLI credential storage with keyring fallback); `claude[bot]`
review items #1, #2, #3.

## Problem

`flox auth login --insecure-storage` writes the FloxHub token to `flox.toml` in plain
text, as intended. But on the **next** invocation of any `flox` command,
`resolve_credential_into` sees a plain-text token and opportunistically migrates it into
the OS keyring, printing "Moved your FloxHub credential from plain text into your system
keyring." There is no persisted signal of the user's choice, so `--insecure-storage` is
effectively a one-shot setting that is reverted on the next command. The flag's documented
intent — "store the token in plain text instead of the OS keyring" — is not honored beyond
the login command itself.

Two adjacent defects in the same migration/warning code (from the `claude[bot]` review)
are folded into this work because they touch the exact lines being changed:

- **#1** — In `resolve_credential_into`, the `keyring.set(..).is_ok() &&
  plaintext.remove().is_ok()` short-circuit returns `ResolveOutcome::Unchanged` when the
  keyring write succeeds but the plaintext removal fails. The state *did* change (the token
  is now in the keyring), no banner or warning is shown, and the next invocation re-attempts
  the migration — a silent retry loop, with the plaintext secret lingering and shadowing the
  keyring (user file > keyring). `persist_login_token` already handles the equivalent case
  with an explicit `message::warning`; the migration path does not.
- **#3** — The C5 warning added to `persist_login_token` uses a bare `\n` inside a
  single-line string literal. `AGENTS.md` requires `formatdoc!`/`indoc!` for multi-line
  strings; the sibling warning in `auth.rs` already uses `formatdoc!`.

## Goal / desired end state

- A user can choose **persistent** plain-text storage that survives subsequent commands and
  subsequent logins: `flox auth login --insecure-storage`.
- A user can choose **temporary** plain-text storage that does not change their standing
  preference: `flox auth login --insecure-storage --once`.
- The choice is a normal config setting, manageable via `flox config` and merged through the
  usual `/etc/flox.toml` → user → env precedence.
- A plain `flox auth login` honors the standing preference (it never silently changes where
  tokens are stored).
- Migration (`plaintext → keyring`) runs only when the standing preference is `keyring`.
- The migration partial-failure (#1) is no longer silent, and the C5 warning uses
  `indoc!`/`formatdoc!` (#3).

## Design

### Single source of truth: a config field

A new enum field on `FloxConfig`, defined and used exactly like the existing
`floxhub_authn_mode`:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TokenStorageMode {
    /// Store the token in the OS-native keyring (default).
    #[default]
    Keyring,
    /// Store the token in plain text in flox.toml.
    Plaintext,
}

// in FloxConfig (flox.toml key: floxhub_token_storage):
#[serde(default)]
pub floxhub_token_storage: TokenStorageMode,
```

`TokenStorageMode` is defined in `config/mod.rs` next to the other config enums so the
config module stays self-contained; `credential_store.rs` imports it. The field is the
single source of truth for *where tokens go*, consulted in two places: at login (where to
write) and at migration (whether to move an existing plain-text token).

### Behavior matrix

| Invocation | Writes the setting? | Token stored | Migration on later commands |
|---|---|---|---|
| `flox auth login` (pref = keyring, default) | no | keyring (fallback → plaintext + warn) | n/a |
| `flox auth login` (pref = plaintext, standing) | no | plaintext (honors pref) | skipped |
| `flox auth login --insecure-storage` | set `= plaintext` | plaintext | skipped (pref is plaintext) |
| `flox auth login --insecure-storage --once` | no | plaintext (this time) | re-secured to keyring next command *if the keyring is available* |
| `flox config --delete floxhub_token_storage` | clears it (→ default keyring) | unchanged | next command migrates plaintext → keyring |

`--once` means "store plain text now without changing my standing preference." Because the
standing preference stays `keyring`, the token auto-migrates into the keyring the moment the
keyring is available again (e.g. once a flaky Secret Service recovers). It is not a
long-lived "stay plain text" — that is the persistent flag's job.

### Login flow (`commands/auth.rs`)

`Auth::Login` gains a second flag:

```rust
Login {
    /// Store the token in plain text in flox.toml instead of the OS keyring.
    #[bpaf(long("insecure-storage"))]
    insecure_storage: bool,
    /// With --insecure-storage, do not persist the plain-text preference
    /// (store plain text only for this login).
    #[bpaf(long("once"))]
    once: bool,
}
```

`login_flox(flox, insecure_storage, once)`:

1. Determine the target mode:
   - `insecure_storage` → `Plaintext`.
   - otherwise → the standing preference `config.flox.floxhub_token_storage`.
2. Persist the preference **only** when `insecure_storage && !once`: write
   `floxhub_token_storage = "plaintext"` via the existing `update_config` path.
   (A plain login does not write the setting — it honors whatever is already there.)
3. Store the token:
   - target `Plaintext` → `PlaintextStore::set` (explicit `0600`) **and** best-effort
     removal of any existing keyring entry (so switching keyring → plain text does not leave
     a stale secret in the keyring), then the plain-text warning.
   - target `Keyring` → `persist_login_token` (keyring with plain-text fallback + warning on
     keyring failure; it already removes any lingering plain-text token on keyring success).

`--once` without `--insecure-storage` has no effect (it only modulates the persistence of
the plain-text choice).

### Migration flow (`commands/mod.rs` + `utils/credential_store.rs`)

`resolve_credential_into` gains a `storage: TokenStorageMode` parameter, computed at the
call site from `config.flox.floxhub_token_storage` (mirroring the existing `is_auth0`
parameter so the function stays unit-testable without constructing full configs):

- Migration (`plaintext → keyring`) runs only when `storage == Keyring` (in addition to the
  existing gates: Auth0 mode, `FLOX_FLOXHUB_TOKEN` unset, user-file token present, not the
  prompt/hook flow). When `storage == Plaintext`, migration is skipped entirely.
- The keyring read-fallback (populate `config.flox.floxhub_token` from the keyring when the
  merged value is empty) is unchanged and not gated by `storage` — it is a read-only
  fallback that moves nothing.

Switching back to keyring is `flox config --delete floxhub_token_storage` (or
`--set floxhub_token_storage keyring`). The preference becomes `keyring`, so the next command
migrates the lingering plain-text token into the keyring automatically.

### Folded-in fix #1 — migration partial-failure is no longer silent

The migration block changes from the silent short-circuit to explicit handling:

```rust
if storage == TokenStorageMode::Keyring
    && !env_set
    && let Ok(Some(token)) = plaintext.get()
{
    if keyring.set(&token).is_ok() {
        match plaintext.remove() {
            Ok(()) => return ResolveOutcome::Migrated,
            Err(e) => {
                // Keyring already holds the token; do not silently loop.
                tracing::warn!("could not remove the plain-text credential after migrating it to the keyring: {e}");
                message::warning(indoc! {"
                    Stored your credential in the system keyring, but could not remove the
                    existing plain-text credential from flox.toml.
                    Remove the 'floxhub_token' line from flox.toml so it does not shadow the
                    keyring."});
                return ResolveOutcome::Unchanged;
            },
        }
    }
    return ResolveOutcome::Unchanged;
}
```

The token is in the keyring after a successful `keyring.set`, so the user is warned (with a
remediation step) instead of getting a silent, repeating no-op. The retry on subsequent
commands still happens (the plain-text file is unchanged), but it is now accompanied by the
warning rather than being silent.

### Folded-in fix #3 — `indoc!` for the C5 warning

The warning in `persist_login_token` switches from a `\n` literal to `indoc!`, matching the
`formatdoc!` warning already in `auth.rs`.

### `flox auth status`

When the standing preference is `Plaintext`, `status` adds one line stating that the
storage preference is set to plain text and naming the command to revert
(`flox config --delete floxhub_token_storage`), so a user understands why their token is not
being migrated. Exact wording follows the message conventions in `AGENTS.md`. This is
additive to the existing source-reporting lines (keyring / plain text / env).

## Testing (TDD — each test watched fail before the fix)

Unit tests on `CredentialStoreImpl::Mock` and a temp config dir, mirroring the existing
`credential_store.rs` tests:

- Login honors a standing `plaintext` preference: stores plain text, never writes the keyring.
- `--insecure-storage` persists the setting; a subsequent `resolve_credential_into` does
  **not** migrate (storage == plaintext).
- `--insecure-storage --once` does **not** persist the setting; a subsequent
  `resolve_credential_into` migrates (keyring available) and leaves plain text untouched
  (keyring unavailable).
- `resolve_credential_into` skips migration when `storage == Plaintext`.
- After `floxhub_token_storage` is cleared/keyring, `resolve_credential_into` migrates the
  plain-text token into the keyring.
- #1: keyring write succeeds + plain-text remove fails → warning emitted, no silent loop
  (assert the warning path is taken; the token is present in the keyring).
- `TokenStorageMode` parses from `flox.toml` (`floxhub_token_storage = "plaintext"`) and
  defaults to `keyring` when absent — exercised through `Config::parse` precedence tests.

## Scope / YAGNI (explicitly out)

- **No `--secure-storage` flag.** Reverting is `flox config --delete floxhub_token_storage`,
  which also re-secures the token on the next command.
- **No per-environment or time-boxed storage.** `TokenStorageMode` stays two-valued
  (`keyring` | `plaintext`); a third backend can be added later without changing this design.
- **No change to the keyring read-fallback** or to the token's runtime representation
  (`Option<String>` → `FloxhubToken` → `AuthContext`).

## Open questions

None. All design forks (persistent default for the bare flag, config-setting mechanism,
enum field shape, plain-login honors the standing preference) were resolved during
brainstorming.
