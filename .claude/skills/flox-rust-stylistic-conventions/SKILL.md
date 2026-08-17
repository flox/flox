---
name: flox-rust-stylistic-conventions
description: Use when editing Rust in flox/flox. Encodes review-validated stylistic conventions — rules where the original code works but reviewers prefer a different shape. Covers naming conventions, formatting, imports, message wording style, and other taste-driven choices. Mined from 944 review comments across 216 PRs (8 months). Complements AGENTS.md and lives alongside `flox-rust-review`.
---

The flox team prioritizes legibility of intent over brevity: names should describe what a thing *does* or *represents*, not how it is triggered or which vendor powers it; user-facing strings are stretched past the 80-character source limit rather than broken mid-thought; warnings are reserved for conditions the user can act on; and every repeated literal or pattern gets extracted into a named constant or helper the moment it appears twice. Reviewers consistently nudge toward specificity — the right domain constant, the right qualifier on visibility, the right term used uniformly across all call sites.

---

## Naming

**Use the `str_to_x` pattern for query-parameter parsers in `flox-catalog`; do not inline the parsing.**

The file already has `str_to_catalog_name`, `str_to_package_name`, etc. A new parser for a system field must be named `str_to_system`, not `PackageSystem::from_str` inlined at the call site.

```rust
// before
let system = api_types::PackageSystem::from_str(system).map_err(|_| { ... })?;

// after
let system = str_to_system(system)?;
```

Evidence: PR #4156. (reinforces AGENTS.md §Naming new helpers)

---

**Name a function by what it does, not what triggered it; when a name is opaque, add a doc comment.**

`gather_services_for_flag` became `services_to_start`. `check_build` became `check_build_already_recorded` with a doc comment explaining it checks for duplicate builds in the catalog before spending time on a Nix build.

Evidence: PR #4152, PR #4156. (reinforces AGENTS.md §Conventions)

---

**Name CLI positional arguments by their actual type (`installable`, `attrpath`), not an approximate concept (`expression`).**

The positional argument in `#[bpaf(positional("expression"))]` was renamed to `"installable"` once the parameter accepted a flakeref + attrpath combined form.

Evidence: PR #3599. (reinforces AGENTS.md §Conventions)

---

**Do not prefix test functions with `test_`.**

`#[test]` and the `#[cfg(test)]` module already identify them. Name tests descriptively for what they verify: `gather_repo_meta_no_upstream_suggests_set_upstream`, not `test_gather_repo_meta`.

Evidence: PR #4165. (reinforces AGENTS.md §Test naming)

---

**Use generic authentication terminology in type and variant names; do not surface provider names (`Auth0`, `Auth0Mode`) in the API.**

The config-level type was renamed from `Auth0Mode` to `AuthnMode`. The distinction between `AuthnMode` (config-level policy) and `AuthContext` (runtime-materialized credential) should be visible in the name. Use a single term — `auth_context` — consistently across function parameters, struct fields, and test helpers.

Evidence: PR #4172. (reinforces AGENTS.md §Conventions)

---

**Name structs by their purpose, not their implementation; prefer `DiffSerializer` over `ActivationDiff`.**

When a struct serializes or transforms data rather than representing a domain concept, pick a name that describes its role (`DiffSerializer`) rather than the domain entity it was derived from (`ActivationDiff`).

Evidence: PR #4202. (reinforces AGENTS.md §Conventions)

---

**Use domain-specific constants instead of magic numbers.**

Replace bare integer literals like `2` for stderr with `nix::libc::STDERR_FILENO`.

Evidence: PR #3801.

---

**Extract repeated string literals into named constants; do not repeat them across call sites.**

When the same string appears more than once, define a constant. This is especially true for environment variable names and fixed command strings used in tests.

Evidence: PR #4231. (reinforces AGENTS.md §Conventions)

---

**Use the `COMMON_NIXPKGS_URL` constant as the default nixpkgs flake reference, not the bare string `"nixpkgs"`.**

```rust
// before
let flake_ref = nixpkgs_flake.as_deref().unwrap_or("nixpkgs");

// after
let flake_ref = nixpkgs_flake.as_deref().unwrap_or(&COMMON_NIXPKGS_URL);
```

Evidence: PR #3599. (reinforces AGENTS.md §Conventions)

---

**Keep shell-specific helpers in their respective modules; extract into subdirectories once a module grows large.**

Shell helper code (bash, zsh, fish) belongs in per-shell modules, not a single shared file. When a single module exceeds a reasonable size, split into a subdirectory with `mod.rs`.

Evidence: PR #4231. (reinforces AGENTS.md §Conventions)

---

**Document non-obvious structs explaining what they represent, their invariants, and their relationship to adjacent types.**

A struct like `FloxmetaBranch` that is not self-explanatory needs a doc comment covering: what it wraps, what invariants hold, and how it differs from related types (`Generations`, `CoreEnvironment`).

Evidence: PR #3813. (reinforces AGENTS.md §Conventions)

---

**Document trait methods, not trait implementations; omit doc comments from `impl` blocks that would duplicate the trait's own docs.**

Evidence: PR #4076. (reinforces AGENTS.md §Conventions)

---

**Use semantic names in docs and user-facing text: "source reference" not "flake reference" when the value does not have to be a flake.**

Evidence: PR #4183. (reinforces AGENTS.md §Conventions)

---

## Formatting

**Use `formatdoc!` or `indoc!` for multiline formatted strings; never `format!` with `\` line continuations or raw strings.**

```rust
// before
let msg = format!(r#"
    First line {foo}
    Second line {bar}
"#);

// after
let msg = formatdoc! {"
    First line {foo}
    Second line {bar}
"};
```

Evidence: PR #4156, PR #4165. (reinforces AGENTS.md §Conventions)

---

**Keep source code lines at 80 characters; stretch past 80 for user-facing string content rather than breaking mid-thought.**

These are not contradictory: the 80-character limit governs code structure; user-visible messages are stretched because line breaks in `formatdoc!` appear verbatim in the terminal. When a message fits on one line under ~80 chars, keep it there. When it cannot, use `formatdoc!` and place natural sentence breaks at the wrapped line rather than source-column breaks.

Evidence: PR #3646. (reinforces AGENTS.md §User-facing string literals)

---

**Use `pub(crate)` or `pub(super)` instead of bare `pub` for functions and constants that are not part of a stable public API.**

Bare `pub` implies a commitment to external callers. Module-internal helpers, constants, and crate-only functions should use `pub(crate)` or `pub(super)`.

```rust
// before
pub const SOME_INTERNAL_CONSTANT: &str = "...";

// after
pub(crate) const SOME_INTERNAL_CONSTANT: &str = "...";
```

Evidence: PR #4172, PR #3988. (reinforces AGENTS.md §Conventions)

---

**Break long method chains and assignments across lines at natural boundaries.**

Evidence: PR #4093.

---

**Do not extract a helper function for a single-use operation; inline the code.**

Helpers are for reuse. When a function is only ever called from one place, inline it rather than creating an indirection that obscures the reader's path through the code.

Evidence: PR #4140. (reinforces AGENTS.md §Conventions)

---

**Add explanatory comments when non-obvious patterns (manual symlinks, recursion, catalog heuristics) are maintained by hand.**

If the structure cannot be inferred from reading adjacent code, add a comment saying why.

Evidence: PR #3960, PR #4122.

---

## Imports

**Use workspace dependency versions in `Cargo.toml`; do not pin a version separately in a member crate when the workspace root already declares it.**

Evidence: PR #3939.

---

**Import tracing macros (`warn!`, `debug!`, `info!`, `instrument`) at the module level with `use tracing::...`; do not qualify them with `tracing::` at each call site when the module already uses the crate.**

Evidence: PR #4156. (reinforces AGENTS.md §use guidelines)

---

**Private modules implicitly narrow function visibility; understand this before adding `pub` to items inside a `mod` block that is not itself `pub`.**

A `pub fn` inside a private `mod` is still invisible outside the file. Rely on module privacy instead of redundant `pub(crate)` annotations where the module already provides the restriction.

Evidence: PR #4172.

---

## User-facing Message Style

**Error messages must name the operation that failed, not a copy-pasted operation from nearby code.**

When `update_catalogs` was added by copying `import_nixpkgs`, the error strings still said "Cannot import from nixpkgs…". Each operation must have its own wording describing what *it* does.

```rust
// before (copy-paste)
bail!("Cannot import from nixpkgs in an environment on FloxHub.")

// after (correct)
bail!("Cannot update catalogs in a managed environment on FloxHub.")
```

Evidence: PR #3969. (reinforces AGENTS.md §User-visible message syntax)

---

**Error messages must describe the condition accurately; always verify what triggers the message before writing it.**

Evidence: PR #3649, PR #4165. (reinforces AGENTS.md §Understand semantics before rewriting messages)

---

**Include actionable next steps at the end of error and restriction messages.**

When a user hits a restriction ("Cannot change the owner of an environment already pushed to FloxHub."), the next line should tell them what to do instead.

Evidence: PR #3649. (reinforces AGENTS.md §User-visible message syntax)

---

**Emit warnings only for conditions the user can act on; suppress noise from automatic actions.**

When a service auto-starts on every activation, do not warn on every activation that "no services are defined." Reserve `message::warning` for `--start-services` being explicitly requested by the user. Auto-triggered code paths should fail silently or log at `debug!` level.

Evidence: PR #4152. (reinforces AGENTS.md §User-visible message syntax)

---

**Keep user-facing messages concise when the state is unchanged; do not describe a non-event verbosely.**

Evidence: PR #3869. (reinforces AGENTS.md §User-visible message syntax)

---

**Do not warn twice for the same condition; merge the user-visible warning into one clear sentence.**

If a check fails, emit one `message::warning` with a plain-language explanation (e.g., "Unable to check if already published — continuing with build and publish."), and separately log the internal detail at `warn!` in tracing.

Evidence: PR #4156. (reinforces AGENTS.md §User-visible message syntax)

---

**Show error messages with the full joined path (`{dir}/pkgs`), not path components the user must concatenate mentally.**

Evidence: PR #4096. (reinforces AGENTS.md §User-visible message syntax)

---

**Show relative paths in error messages, not absolute paths from the Nix store or build checkout.**

Evidence: PR #4102. (reinforces AGENTS.md §User-visible message syntax)

---

**Progress messages use gerund form ("Building…", "Publishing…") with quoted context details.**

Match the form used by existing `message::*` calls throughout the CLI. Check existing patterns before inventing a new format.

Evidence: PR #4140. (reinforces AGENTS.md §CLI output conventions)

---

**In documentation, present general forms before shorthand; introduce the structured syntax before URL syntax.**

When documenting a configuration format that has both a `type`-based structured form and a URL shorthand, document the structured form first because the URL form is derived from it.

Evidence: PR #4183.

---

**Provide complete documentation for configuration options: list possible values, explain the semantics of each, and describe the default.**

Evidence: PR #4198. (reinforces AGENTS.md §Conventions)

---

---

## Where reviewers most often nudge style

1. **`formatdoc!`/`indoc!` discipline** — `format!` with `\` continuations or raw strings appear repeatedly; reviewers catch them in almost every PR.
2. **Message accuracy** — copy-pasted error strings that describe the wrong operation, and messages that describe internal tool state rather than product-level meaning.
3. **Naming specificity** — function names that describe a trigger (`gather_services_for_flag`) rather than a result (`services_to_start`), and provider-specific terms leaking into the type API (`Auth0Mode`, `Auth0`).
4. **Visibility narrowing** — bare `pub` on helpers and constants that only need `pub(crate)` or `pub(super)`.
5. **Warning noise** — warnings emitted unconditionally by auto-triggered code paths, and duplicate warnings for the same condition.
