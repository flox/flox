# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Flox is a virtual environment and package manager built on Nix. It creates portable, reproducible developer environments that can be shared across the software lifecycle.

**Languages:** Rust (CLI), Nix (packaging/environments), C++ (Nix plugins), Bash (activation scripts)

## Development Setup

All tools (`just`, `cargo`, `rustc`, `bats`, etc.) are provided by the Nix dev
shell. They are not available on bare PATH.

```bash
nix develop                    # Enter interactive dev shell
```

**For agents and non-interactive use**, commands need access to the dev shell.
If `IN_NIX_SHELL` is set, you are already inside a dev shell and can run
commands directly. Otherwise, wrap every command with `nix develop -c`:

```bash
# Already in dev shell (IN_NIX_SHELL is set) — run directly:
just build
cargo clippy --all
git push origin my-branch

# Not in dev shell — wrap with nix develop -c:
nix develop -c just build
nix develop -c cargo clippy --all
nix develop -c git push origin my-branch
```

**This includes git operations** — `git push` and `git commit` trigger
pre-commit/pre-push hooks that depend on tools (clippy, rustfmt, treefmt)
provided by the dev shell.

**Check first** — inspect the `IN_NIX_SHELL` environment variable to determine
if wrapping is needed. If unset, the `nix develop -c` prefix is required or
commands will fail with "command not found" errors.

## Common Commands

All commands below assume you are inside `nix develop` or prefixed with
`nix develop -c`.

```bash
# Building
just build                     # Build flox and all subsystems (debug)
just build-release             # Build optimized release version
just build-cli                 # Build only CLI (faster for Rust-only changes)

# Running
./target/debug/flox --help     # Run built binary
cargo run -p flox -- <args>    # Run via cargo

# Testing
just test-all                  # Full test suite (nix-plugins, unit, integration)
just test-cli                  # CLI tests only (impure + integration)
just unit-tests                # Unit tests
just impure-tests              # Unit tests with extra-tests feature
just integ-tests               # Integration tests (bats)
just unit-tests "test_name"           # Run specific unit test
just integ-tests usage.bats                       # Run specific integration test file
just integ-tests -- --filter-tags tag             # Run integration tests by tag
just integ-tests -- --filter regex                # Run integration tests by name
just integ-tests activate.bats -- --filter regex  # Run integration tests, filtering by both test file and test name. This is faster when wanting to run tests in a single file, because bats doesn't have to filter through all the tests in other files

# Formatting and Linting
just format                    # Format all code
cargo fmt                      # Format Rust
cargo clippy --all             # Lint Rust
treefmt -f nix .               # Format Nix
pre-commit run -a              # Run all linters
```

## Architecture

### Rust Workspace (`cli/`)

| Crate | Purpose |
|-------|---------|
| `flox` | Main CLI binary, command implementations |
| `flox-rust-sdk` | Core SDK: data structures, models, providers |
| `flox-core` | Low-level utilities (activations, paths, versions) |
| `flox-activations` | Environment activation binaries and process monitoring |
| `catalog-api-v1` | Catalog API client (generated from OpenAPI) |
| `flox-test-utils` | Shared test helpers |
| `mk_data` | Test data generator |
| `xtask` | Build tasks (schema generation) |

### Key Directories

| Directory | Purpose |
|-----------|---------|
| `cli/flox/src/commands/` | CLI command implementations |
| `cli/flox-rust-sdk/src/models/` | Environment models (managed, remote, project) |
| `cli/flox-rust-sdk/src/providers/` | Service providers (catalog, packages, etc.) |
| `cli/tests/` | Integration tests (32 bats files) |
| `nix-plugins/` | C++ Nix plugins (Meson build) |
| `pkgs/` | Nix package definitions |
| `assets/activation-scripts/` | Shell activation scripts |
| `test_data/` | Mock responses and test fixtures |

## ld-floxlib (LD_AUDIT shared library)

`ld-floxlib/ld-floxlib.c` is a C shared library loaded via
`LD_AUDIT` to resolve dynamic libraries from Flox environments.

### GLIBC version binding requirement

**Every libc function used in this file MUST have an explicit
`.symver` asm binding** in both architecture blocks:
- `__aarch64__`: `GLIBC_2.17`
- `__x86_64__`: `GLIBC_2.2.5`

When adding or changing any C standard library call (`malloc`,
`strlen`, `strdup`, etc.), add the corresponding `__asm__`
statement in both `#if` blocks at the top of the file. Missing
bindings cause the library to link against a newer GLIBC version
than the target host supports, breaking portability.

### Build and test

```
cd ld-floxlib && make clean && make && make test
```

## Testing

### Mock Data Generation

Mock catalog responses are generated against local floxhub services. See `CONTRIBUTING.md` for details on regenerating mocks.

## Debugging Activation Scripts

Set `FLOX_ACTIVATE_TRACE=1` to trace activation script execution:

```bash
nix build
FLOX_ACTIVATE_TRACE=1 result/bin/flox activate [args]
```

## Conventions

- **Rust style:**
  - Follow existing style and Rust idioms
  - Use early returns from functions and functional programming style; don't use nested conditionals
  - Structs should derive `Clone` and `Debug`
  - Use structured log and tracing fields; don't interpolate variables into single strings
  - Use `assert_eq!` on entire structs in tests so that it's easier to debug failures and catch new fields; don't `assert!` or `assert_eq!` on individual fields
  - Add `use` statements to modules; don't inline absolute paths and don't add to nearest function
  - Always update `use` statements when moving code between modules; don't re-export existing names
  - **Error handling architecture:**
    - When improving error messages, first understand the existing
      error type hierarchy before adding string-matching at call
      sites. Extend error enums with new variants rather than
      parsing `.to_string()` output.
    - The git provider layer has a classification pattern:
      `GitCommandError` → `GitRemoteCommandError` (with typed
      variants like `AccessDenied`, `Diverged`, `RefNotFound`).
      New failure modes should be added as variants here, not
      detected by string matching downstream.
    - Credential sanitization, access-denied detection, and similar
      cross-cutting concerns belong in `Display` impls or `From`
      conversions on the error types, not sprinkled at individual
      call sites.
  - **Provider traits and associated types:**
    - Before defining a provider trait, ask whether alternative
      implementations (including mocks) are realistic. If not, a
      concrete type is simpler and more honest.
    - Avoid associated types on provider traits unless
      alternative implementations are realistic. They make
      bounds harder to write and in practice often constrain
      consumers to exactly one implementation.
    - If a provider trait already has associated types, don't
      constrain them in consumers (e.g.,
      `impl GitProvider<GetOriginError = …>`). If something only
      works with one implementation, take the concrete type
      directly rather than hiding that fact behind a pinned trait
      constraint.
    - For non-provider traits where associated types are
      semantically meaningful (e.g.,
      `impl IntoIterator<Item = X>`), constraining them is
      correct and expected.
  - **Understand semantics before rewriting messages:** Before
    changing an error message, verify what condition actually
    triggers it. The message must describe what is actually wrong,
    not an approximation inferred from a surface reading of the
    code.
  - Use `formatdoc!` (from `indoc`) for multiline formatted
    strings rather than `\n\` line continuations in function
    bodies. Proc-macro attributes (`#[error(...)]`,
    `#[bpaf(...)]`) require string literals and cannot use
    macros.
  - **User-facing string literals:** Prefer stretching past the
    line-width limit rather than breaking messages with `\`
    continuations. The output the user sees matters more than
    source line length. Quote suggested commands with single
    quotes (e.g., `'git push'`).
  - **Test naming:** Do not prefix test functions with `test_`.
    The `#[cfg(test)]` module and `#[test]` attribute already
    identify them as tests. Name tests descriptively for what
    they verify (e.g.,
    `gather_repo_meta_no_upstream_suggests_set_upstream`).
- **Commits:** Conventional commits format (`feat:`, `fix:`, `chore:`, etc.). Use `cz commit` for interactive commits
- **Rust 2024 edition** for main crates

## Manifest usage (`flox-manifest` crate)

The `flox-manifest` crate uses a type-state pattern (`Manifest<S>`) to enforce
correct manifest lifecycle at compile time. Follow these rules strictly.

- **New schema version for shape changes** - any change to the manifest schema
  (adding, removing, or renaming fields/sections/tables) requires creating a
  new schema version. Never modify an existing schema version's structure.

- **Adding new schemas** - copy the latest `flox-manifest/src/parsed/v*.rs` to
  a new version file and duplicate modified leaf types. Unmodified types
  continue to live in `parsed::common` or their respective version.

- **Always use `Manifest` constructors** - don't pass manifest content as
  `String` or deserialize into inner types directly (e.g.
  `toml_edit::de::from_str::<ManifestLatest>()`). Any manifest read from disk
  or received as text must be migrated. Use the typed constructors:
  - `Manifest::read_typed(path)` / `Manifest::parse_toml_typed(s)` →
    `Manifest<Validated>`
  - `Manifest::read_and_migrate(path, lockfile)` /
    `Manifest::parse_and_migrate(s, lockfile)` → `Manifest<Migrated>`
  - `Manifest::parse_json(s)` for lockfile-embedded manifests →
    `Manifest<TypedOnly>`

- **Never serialize manifests by hand** - don't use
  `toml_edit::ser::to_string()` on inner types. Use
  `manifest.as_writable().to_string()` or
  `manifest.as_writable().write_to_file(path)`, which handle schema version
  selection and format preservation.

- **Outside `flox-manifest`, operate on `ManifestLatest`** - do not introduce
  or expand adapter traits like `CommonFields` so callers can pretend all
  schema versions are interchangeable. The intended model is: migrate to the
  latest schema, then operate on `ManifestLatest`.

- The pattern for `PackageLookup` and `SchemaVersion` doesn't quite match the
  pattern we want to use for operating on `ManifestLatest`, but for now we'll
  keep using the `PackageLookup` trait.

- **Use lockfile migration helpers** - when reading manifest data from a
  `Lockfile`, prefer `lockfile.migrated_manifest()` for the merged manifest and
  `lockfile.migrated_user_manifest()` for the user-authored manifest instead of
  calling `lockfile.manifest.migrate_typed_only(...)` directly at call sites.

- **Tests: use test helpers** (behind `feature = "tests"`):
  - `flox_manifest::raw::test_helpers`: `mk_test_manifest_from_contents()`,
    `empty_test_migrated_manifest()`
  - `flox_manifest::test_helpers`: `with_latest_schema("body")` to prepend
    the correct schema version to TOML content strings

## IDE Setup

For rust-analyzer, add to `.vscode/settings.json`:

```json
{
  "rust-analyzer.linkedProjects": ["${workspaceFolder}/Cargo.toml"],
  "rust-analyzer.cargo.features": ["extra-tests"]
}
```
