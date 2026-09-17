---
name: flox-rust-review
description: Use when editing Rust in flox/flox. Encodes review-validated rules where the original code was wrong, buggy, or insufficient — error handling, type safety at boundaries, semantic correctness, testing patterns, provider-trait design, manifest-usage discipline, panic discipline. Mined from 944 review comments across 216 PRs (8 months). Complements AGENTS.md and lives alongside `flox-rust-stylistic-conventions`.
---

These rules are derived from substantiated PR review findings — cases where reviewers identified a concrete bug, missing correctness invariant, or structural flaw that the author subsequently fixed. Each rule names the specific Flox identifier, module, or pattern where the problem arose. Generic style preferences live in the sibling `flox-rust-stylistic-conventions` skill.

---

## Error handling

**Add typed error variants for new failure modes; remove match arms for errors that can no longer occur.**
When a new operation can fail in a way not yet represented in the error enum, add a variant rather than matching on `.to_string()` output or returning a generic string. Symmetrically, when a code path is removed, delete the corresponding match arm — dead arms mislead readers. (PRs #3646, #3673, #3794)

**Propagate the full error chain; never flatten with `display_chain()` or `.to_string()` at intermediate layers.**
`Box<dyn std::error::Error + Send + Sync>` or the `thiserror` `#[source]` attribute preserve context that callers need for logging and further classification. Converting to a string at an intermediate layer discards the chain permanently. Reserve `.display_chain()` for the point of final user presentation. (PRs #3673)

**Check `.status.success()` on every `Command` invocation; do not silently swallow non-zero exits.**
A process exit that goes undetected produces silent data corruption or misleading success messages. `output.status.success()` is the correct check; inspecting only `stdout` is insufficient when the command could fail. (PR #3794)

**Map `NotFound` (ENOENT) at exec boundaries to an explicit user-facing error, not a generic I/O error.**
When running an external binary with `Command::new`, the OS returns `ErrorKind::NotFound` if the binary is missing. Detect this at the call site and return a domain-specific error that names the missing tool, rather than propagating raw `std::io::Error` or silencing the failure. (PR #3803)

**Cover all `ConcreteEnvironment` variants in match arms; do not write arms only for `Path`.**
`ConcreteEnvironment` has `Path`, `Managed`, and `Remote` variants. Exhaustive match arms are not always enforced by the compiler when the match is over a method result; audit each new match to confirm all variants are handled correctly. (PR #3599)

**Never silence errors from auth validation; surface them as distinct typed errors.**
Auth-related code paths are particularly prone to swallowing errors (`let _ = ...` or ignoring `Err`) under the assumption that validation failure is not actionable. Each validation failure is an actionable error that should be returned to the caller. (PR #4047)

---

## Type safety

**Use `&Url` (or `url::Url`) instead of `&str` for URL parameters through the entire call chain.**
Accepting `&str` at any intermediate function allows callers to pass unvalidated strings and defers parse errors to unexpected sites. Parse once at the entry point (CLI argument parsing or API deserialization) and pass `&Url` through the chain. (PRs #4156, #4172)

**Use `Option<&str>` (borrowed) instead of `Option<String>` (owned) when the data is already stored in `self`.**
Returning or accepting `Option<String>` forces an unnecessary clone when the backing data lives in the struct. `Option<&str>` with a lifetime tied to `&self` avoids the allocation and makes ownership explicit. (PR #4172)

**Use `NixFlakeRef` instead of `String` for Nix flake references; parse at CLI/API entry points.**
`NixFlakeRef` is the domain type for flake references. Passing raw `String` through business logic means callers can never distinguish a flake ref from an arbitrary string, and validation is silently skipped. Parse at the outermost boundary (arg parsing or response deserialization) and propagate the typed value. (PRs #3599, #4156)

**Use `Shell` (the `shell_gen` enum) instead of `&str` for shell-type parameters.**
The `Shell` enum from the `shell_gen` crate enumerates the shells Flox supports. Passing `&str` requires every consumer to parse or match strings, creating divergence risk. Accept `Shell` at function boundaries. (PR #4231)

**Encode auth-mode availability in an `AuthContext` enum type, not in stringly-typed or nullable fields.**
The distinction between `Auth0(Option<Token>)` and `Kerberos(...)` auth modes is a domain invariant. Represent it as an enum variant so the compiler enforces correct handling. A missing token in Kerberos context is not an error; encoding this in a nullable field or `bool` flag makes it invisible. (PRs #4047, #4172)

**Parse installable descriptor outputs (`^`) before version (`@`); the split order matters.**
In Nix installable syntax, `pkg^output@version` must be split on `^` first to separate the output selector from the rest, then on `@` for the version. Reversing the order produces silently wrong parses that pass tests but fail on real inputs. (PR #3864)

**Do not derive `Ord` on a type unless it is actually stored in an ordered collection (`BTreeSet` / `BTreeMap`).**
`Ord` implies a total ordering that must be semantically meaningful. Deriving it for convenience (e.g., to silence a compiler warning) imposes an arbitrary ordering that can mislead callers and may later be relied on incorrectly. Add `Ord` only when the collection use requires it. (PR #3864)

---

## Semantic correctness

**Use NUL-separated file lists (`find -print0` / `xargs -0` / `--null` flags) when passing filenames to shell commands.**
File paths can contain spaces, newlines, and glob characters. Passing them as newline-separated or space-delimited strings to shell commands causes silent misparses and potential injection. Use `\0`-delimited lists end-to-end. (PR #4191)

**Kerberos auth with a missing FloxHub token is not an error; do not treat `NixAuth { floxhub_token: None }` as a failure.**
In Kerberos environments the `floxhub_token` field is legitimately absent. Returning an error when the token is `None` in this branch causes spurious failures in Kerberos deployments. Guard the error return on the auth mode, not on the presence of the token alone. (PR #4172)

**Refactor shared activation logic (e.g., `apply_activation_env`) rather than duplicating it across call sites.**
`apply_activation_env` and `collect_activate_exports` in `flox-activations` are the canonical implementation of activation environment setup. Duplicating logic adjacent to these functions diverges over time and causes subtle behavioral differences. Extract shared logic into a helper that both call sites invoke. (PR #4202)

**Keep user-visible message construction in the CLI layer, not in the SDK.**
`flox-rust-sdk` and `flox-core` are library crates. Embedding user-facing strings (messages, hints, formatting) inside SDK code couples the library to a specific presentation layer and prevents reuse. Move message construction to the command handler in `flox/src/commands/`. (PR #4094)

**Do not return stale data when an upstream operation fails; propagate the error.**
When an operation that refreshes or mutates state fails, the function must propagate the error rather than returning the pre-operation state. Returning stale data silently makes the caller believe the mutation succeeded. (PR #3599)

---

## Testing

**Use `&mut impl Write` as the output sink in renderers so they can be unit-tested without spawning a process.**
Functions that produce terminal output should accept a `&mut impl Write` parameter rather than writing to `stdout` directly. This allows unit tests to pass a `Vec<u8>` and assert on exact output without capturing process output. (PR #3695)

**Use `assert_eq!` on the entire struct, not on individual fields.**
Asserting on individual fields means new fields added to the struct are silently excluded from comparison. `assert_eq!` on the whole struct catches regressions in newly added fields automatically and produces diffs that show the full context of a failure. This is also stated in AGENTS.md.

**Avoid non-deterministic test failures from unstable collection ordering; sort or use `BTreeSet` before asserting.**
`HashMap` and `HashSet` iteration order is not guaranteed. Tests that assert on output derived from unordered collections can pass or fail depending on hash randomization. Sort the collection or use `BTreeSet`/`BTreeMap` in test assertions. (PR #3951)

**Name tests descriptively for what they verify; do not prefix with `test_`.**
The `#[test]` attribute already identifies a function as a test. The name should describe the invariant or scenario, e.g., `gather_repo_meta_no_upstream_suggests_set_upstream`. This is also stated in AGENTS.md.

---

## Provider traits

**Add `unimplemented!()` bodies for deprecated `MockClient` dispatch methods in `catalog.rs`; do not extend the deprecated pattern.**
`MockClient` in `flox-rust-sdk/src/providers/catalog.rs` uses a dispatch pattern that is being phased out. When a trait method must be satisfied but the implementation is intentionally unused, use `unimplemented!()` and note it in the PR rather than adding a working implementation to the deprecated machinery. (PR #4156)

**Avoid associated types on provider traits unless alternative implementations (including mocks) are realistic.**
Associated types on provider traits make bounds harder to write and in practice often constrain consumers to exactly one implementation. If a trait is only ever implemented by one concrete type, define a concrete type or use a simpler trait without associated types. This is also stated in AGENTS.md.

**Take concrete types directly when a consumer only works with one implementation; do not hide that behind a pinned trait constraint.**
Writing `impl GitProvider<GetOriginError = …>` constrains the generic to one implementation while pretending it is generic. This is worse than accepting the concrete type: it is misleading and makes the constraint harder to discover. (AGENTS.md)

---

## Manifest usage

**Never pass manifest content as `String` or deserialize into inner types directly; always use `Manifest<S>` constructors.**
The `Manifest<S>` type-state ensures that manifests pass through migration and validation before use. Bypassing constructors (e.g., calling `toml_edit::de::from_str::<ManifestLatest>()` at a call site) skips migration and produces a manifest that may be in an older schema version than expected. Use `Manifest::read_typed`, `Manifest::parse_toml_typed`, or `Manifest::read_and_migrate` as appropriate. (PR #4076; also in AGENTS.md)

**Outside `flox-manifest`, operate on `ManifestLatest`; add accessor methods to `ManifestLatest` instead of introducing adapter traits.**
Adapter traits like `CommonFields` that abstract over multiple schema versions make callers pretend all versions are interchangeable, which defeats the type-state migration model. After migration, code outside the `flox-manifest` crate should hold `ManifestLatest` and call methods on it. (PR #4094; also in AGENTS.md)

**Never serialize manifests with `toml_edit::ser::to_string()` on inner types; use `manifest.as_writable().to_string()` or `write_to_file(path)`.**
Manual serialization bypasses schema-version selection and format-preservation logic. The `as_writable()` method handles both. (AGENTS.md)

---

## Logging and tracing

**Use structured `tracing` fields; do not interpolate variables into a single message string.**
`tracing::debug!(token = %token, "auth resolved")` is correct; `tracing::debug!("auth resolved: {token}")` is not. Structured fields are queryable and filterable; interpolated strings are opaque to tracing subscribers. (AGENTS.md; reinforced by PR #4172)

**Emit a `tracing::debug!` (or `tracing::trace!`) span at each auth-mode branch.**
Auth mode selection is a critical control-flow branch for diagnosing production issues. Each branch (Auth0, Kerberos, unauthenticated) should emit a trace event with the selected mode so that log capture makes the decision observable. (PR #4172)

---

## Deprecated patterns

**Do not extend the `apply_activation_env` single-caller pattern; prefer extracting shared helpers.**
As of the PRs reviewed, `apply_activation_env` in `flox-activations` was being called in one place and the logic was being duplicated in an adjacent function. When extending activation behavior, extract the shared logic into a named helper rather than copying the implementation. (PR #4202)

**Do not add implementations to deprecated `MockClient` dispatch methods in `flox-rust-sdk/src/providers/catalog.rs`.**
The `MockClient` mock-dispatch approach in this file is explicitly being phased out. New provider methods should not receive working `MockClient` implementations; use `unimplemented!()` and document why in the PR. (PR #4156)

---

## When in doubt

- **Prefer the typed domain value over `String` at every function boundary.** Parse once at the entry point; propagate `NixFlakeRef`, `Url`, `Shell`, `AuthContext`, `Manifest<S>`, etc.
- **Error variants before error strings.** When a new failure mode appears, extend the error enum before reaching for `.to_string()` or a bare `anyhow` error.
- **Test the whole struct.** `assert_eq!(actual, expected_struct)` catches regressions in new fields that field-by-field asserts miss.
- **Keep user messages in the CLI layer.** SDK crates have no business knowing what the terminal should print.
- **Check the existing error classification hierarchy** (`GitCommandError` → `GitRemoteCommandError`, `ManagedEnvironmentError`, etc.) before adding a new error type or matching on strings.
