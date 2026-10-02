# Conventions for `cli/`

This is the Rust workspace for Flox. All CLI commands, core SDK models, providers, and activation binaries live here. This file is auto-loaded by Claude Code whenever you edit anything under `cli/`, so its rules are active on every PR. They distil the highest-confidence cross-cutting patterns observed across 944 review comments spanning 216 PRs — the most common reasons reviewers asked for changes before approving.

These rules complement, and never override, `AGENTS.md`. Where AGENTS.md and this file give the same guidance, the PR citations below are the supporting evidence. Where only this file speaks to a rule, treat it as a refinement mined from code review that AGENTS.md has not yet absorbed.

## Where to look

| You're editing… | Read first |
|---|---|
| Anywhere in `cli/` | This file + `AGENTS.md` |
| `cli/flox/src/commands/` | `cli/flox/src/commands/CLAUDE.md` |
| `cli/flox-rust-sdk/src/models/environment/` | `cli/flox-rust-sdk/src/models/environment/CLAUDE.md` |
| `cli/flox-rust-sdk/src/providers/` | `cli/flox-rust-sdk/src/providers/CLAUDE.md` |
| Any Rust area — review skills | `.claude/skills/flox-rust-review/SKILL.md` and `.claude/skills/flox-rust-stylistic-conventions/SKILL.md` |

## Cross-cutting rules

**1. Parse to a typed domain value at entry points; propagate it, never the raw string.**
`NixFlakeRef` for flake references, `url::Url` for URLs, `Shell` for shell-type arguments, `AuthContext` for auth mode, `Manifest<S>` for manifests. Accepting `&str` at intermediate layers means every downstream consumer must parse and validate independently, producing divergent string-splitting that breaks on edge cases. Parse once at CLI arg-parsing or API deserialization; propagate the typed value.
Evidence: PRs #3599, #4156, #4172, #4231. Reinforces AGENTS.md §Type safety at function boundaries.

**2. Extend error enums with new variants; never string-match on `.to_string()` at call sites.**
When a new failure mode appears, add a variant to the appropriate error enum (`GitCommandError` → `GitRemoteCommandError`, `ManagedEnvironmentError`, etc.). Matching on `.to_string()` output to detect specific failures ties code to display strings that can change at any time. Remove match arms for conditions that can no longer occur when variants are deleted.
Evidence: PRs #3646, #3673, #4154, #4165. Reinforces AGENTS.md §Error handling architecture.

**3. Preserve the full error chain; never flatten with `display_chain()` or `.to_string()` at intermediate layers.**
`Box<dyn std::error::Error + Send + Sync>` or `thiserror`'s `#[source]` attribute preserve context that callers need for logging and classification. Converting to a string at an intermediate layer permanently discards that chain. Reserve `.display_chain()` for the final user-presentation point.
Evidence: PR #3673. Covered by `flox-rust-review` SKILL.md §Error handling.

**4. Keep user-facing message construction in the CLI layer (`cli/flox/src/commands/`), not in SDK crates.**
`flox-rust-sdk` and `flox-core` are library crates. Embedding formatted user strings there couples the library to a specific presentation layer and blocks reuse. Move message construction to the command handler; let the SDK return typed errors whose `Display` provides a minimal technical description.
Evidence: PR #4094. Covered by `flox-rust-review` SKILL.md §Semantic correctness.

**5. `assert_eq!` on the whole struct, not on individual fields.**
Field-by-field assertions silently exclude new fields added to the struct. `assert_eq!` on the entire expected struct catches regressions in newly added fields automatically and produces diffs that show the full context of a failure.
Evidence: PRs #4076, #4114. Reinforces AGENTS.md §Use `assert_eq!` on entire structs.

**6. Use `Manifest<S>` typed constructors; never pass manifest content as `String` or deserialize inner types directly.**
The type-state pattern ensures every manifest passes through migration and validation before use. Calling `toml_edit::de::from_str::<ManifestLatest>()` at a call site bypasses migration. Outside `flox-manifest`, hold `ManifestLatest`; use `lockfile.migrated_manifest()` helpers rather than calling inner migration methods directly at call sites.
Evidence: PRs #4076, #4094, #4161. Reinforces AGENTS.md §Manifest usage.

**7. Use `formatdoc!` or `indoc!` for multi-line formatted strings; never `format!` with `\` line continuations or raw strings.**
`formatdoc!` handles indentation correctly and makes strings readable in source. The backslash continuation and `r#"..."#` forms are fragile. Exception: proc-macro attributes (`#[error(...)]`, `#[bpaf(...)]`) require string literals.
Evidence: PRs #4156, #4165. Reinforces AGENTS.md §Use `formatdoc!` or `indoc!`.

**8. Use structured `tracing` fields; never interpolate variables into a single message string.**
`tracing::debug!(token = %token, "auth resolved")` is correct; `tracing::debug!("auth resolved: {token}")` is not. Structured fields are queryable and filterable; interpolated strings are opaque to tracing subscribers. Add a `tracing::debug!` at each auth-mode branch (Auth0, Kerberos, unauthenticated) so log capture makes control-flow decisions observable.
Evidence: PR #4172. Reinforces AGENTS.md §Use structured log and tracing fields.

**9. Take the concrete provider type directly when only one implementation exists; do not pin associated types on provider traits.**
`impl GitProvider<GetOriginError = GitCommandGetOriginError>` is a pinned-associated-type constraint that pretends to be generic while only ever accepting one concrete type. This makes bounds harder to write and hides the real dependency. Take `&GitCommandProvider` directly. Avoid associated types on provider traits unless alternative implementations (including mocks) are realistic.
Evidence: PRs #4165, #4172. Reinforces AGENTS.md §Provider traits and associated types.

**10. Do not extend deprecated infrastructure; use `unimplemented!()` and remove tests that depend on it.**
The `MockClient` dispatch pattern in `cli/flox-rust-sdk/src/providers/catalog.rs` is being phased out. When a trait method must be satisfied by the deprecated mock, use `unimplemented!("not supported in MockClient")` rather than adding a working implementation. Delete unit tests that exercised the deprecated mock arm in the same PR.
Evidence: PR #4156. Reinforces AGENTS.md §Deprecated infrastructure.

**11. Do not prefix test functions with `test_`; name them descriptively for what they verify.**
`#[test]` and `#[cfg(test)]` already identify a function as a test. The name should state the scenario or invariant: `gather_repo_meta_no_upstream_suggests_set_upstream`, not `test_gather_repo_meta`. Sorting also improves over time because descriptive names survive refactors.
Evidence: PR #4165. Reinforces AGENTS.md §Test naming.

**12. Use `pub(crate)` or `pub(super)` instead of bare `pub` for functions and constants that are not part of a stable public API.**
Bare `pub` implies a commitment to external callers. Module-internal helpers, constants, and crate-only functions should be narrowed. Drop `pub` entirely on helpers that are only called from within a single module; they are implementation details.
Evidence: PRs #3988, #4172. Covered by `flox-rust-stylistic-conventions` SKILL.md §Formatting.

## When the rules conflict

When the two skills, `AGENTS.md`, and this file disagree, the order of authority is:

1. **`AGENTS.md`** — repo-wide conventions
2. **`cli/CLAUDE.md`** (this file) — Rust cross-cutting refinements
3. **`.claude/skills/flox-rust-review`** and **`.claude/skills/flox-rust-stylistic-conventions`** — review-mined detail
4. **Area-specific `CLAUDE.md`** — most specific wins for that area

One apparent tension worth noting: AGENTS.md says "wrap output at 80 characters," and the stylistic skill says "stretch past 80 for source lines containing user-visible strings." These are not in conflict — 80 chars governs terminal output width; source lines that embed user-facing `formatdoc!` content may be longer because line breaks inside `formatdoc!` appear verbatim to the user. The stylistic skill makes this explicit.

## Source

These rules were mined from 944 review comments across 216 PRs spanning approximately 8 months. The full evidence is in `scripts/pr-analysis/findings/` (key documents: `task9-review.md`, `gap-report.md`). The journey narrative is at `rust-pr-analysis-jouney-01.md` (note: "jouney" is the filename as-committed). Five HTML reports are at the worktree root: `rust-pr-analysis-dashboard-01.html`, `rust-pr-analysis-index-01.html`, `rust-pr-analysis-jouney-01.html`, `rust-pr-analysis-noise-deep-dive-01.html`, and `rust-pr-analysis-pipeline-01.html`.
