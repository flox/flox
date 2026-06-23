# Follow-up: honor the token-storage preference during implicit re-authentication

**Date:** 2026-06-22
**Status:** Deferred — accepted limitation in PR #4422; this document is the design input for the follow-up if/when we decide to close the gap.
**Related:**
- PR #4422 (`feat(auth): persistent token-storage preference with --once`) — introduced `floxhub_token_storage` and gated migration/login on it.
- Design spec: `docs/superpowers/specs/2026-06-22-floxhub-token-storage-preference-design.md`.
- Review item: Codex (`chatgpt-codex-connector[bot]`) **P2** on PR #4422, `cli/flox/src/commands/mod.rs` (the `ensure_auth` re-auth call), "Honor plaintext preference during implicit re-auth".

## Problem

`floxhub_token_storage = "plaintext"` is meant to be a **standing** preference: tokens stay in
plain text in `flox.toml`, and the startup resolver does not migrate them into the OS keyring.
PR #4422 honors this for **explicit** `flox auth login` and for the startup migration.

It is **not** honored on the **implicit re-authentication** path. When a token is expired or
missing and the user runs any command that calls `ensure_auth` interactively (e.g. `flox push`,
`flox publish`, `flox install` into a remote env), `ensure_auth` re-enters the login flow with a
hard-coded `TokenStorageMode::Keyring`:

```rust
// cli/flox/src/commands/mod.rs — ensure_auth()
// Implicit re-authentication stores to the secure default (keyring). The standing
// storage preference is not threaded through the many `ensure_auth` call sites,
// so an implicit re-login does not honor a `plaintext` preference; an explicit
// `flox auth login` does.
auth::login_flox(flox, false, false, TokenStorageMode::Keyring).await
```

### Trace (what actually happens)

1. Preference is `plaintext`; the plain-text token in `flox.toml` has expired.
2. `flox push` → `ensure_auth` → `login_flox(flox, insecure_storage=false, once=false, storage_pref=Keyring)`.
3. `target = Keyring` (because `insecure_storage` is false and the passed `storage_pref` is `Keyring`).
4. `persist_login_token(.., Keyring, ..)` writes the new token to the **keyring** and, on success,
   **removes the plain-text token** from `flox.toml`.
5. The config key `floxhub_token_storage` is still `plaintext` (it is only written by
   `--insecure-storage` without `--once`), but the **token now lives in the keyring**.

### Why it matters

- It silently violates a stated **goal** of the design spec: *"A plain `flox auth login` honors
  the standing preference (it never silently changes where tokens are stored)."* The implicit
  re-auth path is not a "plain `flox auth login`", so it was never in the behavior matrix — but
  from the user's point of view it is a silent relocation of their credential off plain text.
- It produces a mildly contradictory state: `flox auth status` reports both *"Token storage
  preference is set to plain text"* (from the config key) **and** *"Credential stored in your
  system keyring"* (from the live probe).

### Severity / likelihood

**P2, edge case.** Requires the conjunction of: (a) the user set `plaintext`, (b) the token is
expired/missing, and (c) they run an interactive, auth-requiring command (so `Dialog::can_prompt()`
is true). It is not data loss and not a security regression (the keyring is the *more* secure
store); it is a correctness/consistency gap against the user's explicit preference.

## Why this was deferred (the blocker)

`ensure_auth(flox: &mut Flox)` has **no `Config`** in scope, and the token-storage preference lives
in `Config` (`config.flox.floxhub_token_storage`). The naive "just pass the preference" is not a
one-liner because of where `Config` is — and is not — available, and because of crate layering.

### Research: `ensure_auth` call sites and `Config` availability

`ensure_auth` is called from **11 sites**; only **1** has a `Config` in scope.

| Call site | Enclosing function | `Config` in scope? |
|---|---|---|
| `cli/flox/src/commands/publish.rs:152` | `Publish::publish(config: Config, flox: Flox, …)` | ✅ yes |
| `cli/flox/src/commands/install.rs:135` | `Install::handle(self, flox: Flox)` | ❌ no |
| `cli/flox/src/commands/install.rs:628` | `try_create_default_environment_interactive(...)` | ❌ no |
| `cli/flox/src/commands/uninstall.rs:47` | `Uninstall::handle(self, flox: Flox)` | ❌ no |
| `cli/flox/src/commands/upgrade.rs:45` | `Upgrade::handle(self, flox: Flox)` | ❌ no |
| `cli/flox/src/commands/push.rs:55` | `Push::handle(self, flox: Flox)` | ❌ no |
| `cli/flox/src/commands/edit.rs:89` | `Edit::handle(self, flox: Flox)` | ❌ no |
| `cli/flox/src/commands/upload.rs:47` | `Upload::handle(self, flox: Flox)` | ❌ no |
| `cli/flox/src/commands/init/mod.rs:155` | `Init::handle(self, flox: Flox)` | ❌ no |
| `cli/flox/src/commands/mod.rs:1132` | `EnvironmentSelect::to_concrete_environment(&self, flox, …)` | ❌ no |
| `cli/flox/src/commands/mod.rs:1195` | `EnvironmentSelect::detect_concrete_environment(&self, flox, …)` | ❌ no |

Most command `handle()` methods take only `Flox`, not `Config`: `FloxCli::handle()` constructs
`Flox` from `Config` and passes only `Flox` downstream. So "thread the preference to `ensure_auth`"
really means "first make the preference reachable at 10 sites that currently have only `Flox`".

### Research: `Flox` cannot carry `TokenStorageMode` directly (crate layering)

- `TokenStorageMode` is defined in the **`flox` binary crate** (`cli/flox/src/config/mod.rs`).
- `Flox` is defined in **`flox-rust-sdk`** (`cli/flox-rust-sdk/src/flox.rs`).
- `flox-rust-sdk` must **not** depend on the `flox` binary crate (that would be a dependency cycle).
- Therefore `Flox` cannot hold a `TokenStorageMode` field without first moving the enum to a crate
  the SDK already depends on.

**Precedent:** `AuthnMode` (used by `config.flox.floxhub_authn_mode`) lives in `floxhub-client`, and
is **applied at `Flox` construction** rather than carried on `Flox`
(`cli/flox/src/commands/mod.rs`: `AuthContext::from_mode(&config.flox.floxhub_authn_mode, …)`).
The house pattern is: mode/preference stays in `Config`; it is consumed at construction time.

## Options

### Option A — Thread `storage: TokenStorageMode` through `ensure_auth` and its callers

Change `ensure_auth(flox)` → `ensure_auth(flox, storage)`, and pass
`config.flox.floxhub_token_storage` at each call site.

- **Blast radius:** the `ensure_auth` signature + **11 call sites**, but **10 of those call sites
  have no `Config`**. Closing the gap therefore requires plumbing `Config` (or the preference) into
  ~10 command `handle()` methods and the helper functions in `mod.rs`, which in turn means changing
  `FloxCli::handle()`'s dispatch to pass `Config`/preference alongside `Flox` to those subcommands.
- **Pros:** no enum relocation; the preference stays exactly where the spec put it
  (`config/mod.rs`); explicit and greppable.
- **Cons / implications:** the largest change of the three. It touches ~12 files and the central
  command-dispatch surface, expands several public-ish `handle()` signatures, and risks merge
  conflicts with unrelated command work. High review cost for a P2.
- **Risk:** easy to miss a call site or pass the wrong value; each new `ensure_auth` caller in the
  future must remember to plumb the preference.

### Option B — Relocate `TokenStorageMode` to a shared crate and carry it on `Flox`

Move `TokenStorageMode` next to `AuthnMode` in `floxhub-client` (or another crate the SDK already
depends on); `FloxConfig` imports it from there (only the import path changes). Add
`floxhub_token_storage: TokenStorageMode` to `Flox`, set it once at construction in
`FloxCli::handle()` from `config.flox.floxhub_token_storage`, and have `ensure_auth` pass
`flox.floxhub_token_storage`.

- **Blast radius:** ~6 files — define the enum in `floxhub-client`; update imports in
  `config/mod.rs`, `utils/credential_store.rs`, `commands/auth.rs`, `commands/mod.rs`; add one field
  to `Flox` + set it at construction; one-line change at `ensure_auth`. **No change to the other 10
  callers.**
- **Pros:** clean and localized at call sites; `ensure_auth` (and any future caller) gets the
  preference "for free" from `flox`; mirrors how `auth_context`/`AuthnMode` already flow.
- **Cons / implications:** **deviates from the approved spec**, which explicitly says
  *"`TokenStorageMode` is defined in `config/mod.rs` next to the other config enums."* Moving it adds
  a config-shaped enum to a lower layer (though `AuthnMode` is precedent for exactly that). Adds a
  field to the widely-used `Flox` struct.
- **Risk:** low/mechanical, but the relocation is a design decision that should be agreed before
  implementing (it changes the home of a type the rest of the codebase imports).

### Option C — Accept the limitation (chosen for PR #4422)

Leave implicit re-auth using the secure keyring default; document the gap in code and here.

- **Blast radius:** none (already in place; the `ensure_auth` comment documents it and points here).
- **Pros:** zero risk; keeps PR #4422 scoped to what the spec's behavior matrix covers (explicit
  login + startup migration); the keyring is the more secure store, so the fallback is safe.
- **Cons / implications:** the standing `plaintext` preference is not honored for implicit re-auth;
  `flox auth status` can show the contradictory pair noted above until the next explicit action.

## Recommendation

If/when we pursue this, **Option B** is the cleanest: it fixes every `ensure_auth` caller at once,
matches the existing `AuthnMode`/`AuthContext` construction pattern, and is ~6 mechanical files. Its
only real cost is a **deliberate decision to relocate `TokenStorageMode`** out of `config/mod.rs`
(get sign-off first, since it moves a type the codebase imports). **Option A** should be chosen only
if we specifically want to keep the enum in `config/mod.rs`, accepting a much larger,
dispatch-touching change. **Option C** (status quo) is appropriate until a user actually reports the
inconsistency, given the P2 severity.

### Suggested acceptance criteria for the follow-up

- With `floxhub_token_storage = "plaintext"` and an expired token, an implicit re-auth (e.g.
  `flox push`) **keeps** the token in plain text (does not write the keyring, does not remove the
  plain-text file), matching an explicit `flox auth login` under the same preference.
- A unit test exercising `ensure_auth`'s storage decision (or the chosen carrier) for both
  `Keyring` and `Plaintext` preferences.
- `flox auth status` no longer reports "preference is plain text" alongside "stored in your system
  keyring" after an implicit re-auth.
