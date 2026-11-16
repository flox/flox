# AGENTS.md

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
| `flox-events` | v2 telemetry events: envelope types, buffering, sending |
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
| `assets/environment-interpreter/` | Shell activation scripts |
| `test_data/` | Mock responses and test fixtures |

## ld-floxlib

Use [changing-ld-floxlib](.agents/skills/changing-ld-floxlib/SKILL.md)
when changing ld-floxlib.

## Package builder

Use [changing-package-builder](.agents/skills/changing-package-builder/SKILL.md)
when changing build rules or the metadata handoff to the CLI.

## Testing

- Use `assert_eq!` on entire structs in tests so that it's easier to debug failures and catch new fields; don't `assert!` or `assert_eq!` on individual fields
- **Test naming:** Do not prefix test functions with `test_`.
  The `#[cfg(test)]` module and `#[test]` attribute already
  identify them as tests. Name tests descriptively for what
  they verify (e.g.,
  `gather_repo_meta_no_upstream_suggests_set_upstream`).
- **Test behavior that can break, not plumbing:** Do not test
  generated code, the argument parser (bpaf is declarative and
  tested upstream), or serialization round-trips
  (`serialize(x) == serialize(x)`).
- Prefer a type that makes an
  invalid state unrepresentable (`NonZero`, domain types) over a
  runtime guard plus a test for it. Do not add a client mock to
  test a thin wrapper; see the provider-trait rules under Conventions.
- Test when logic is complicated and there are multiple correct codepaths
  through that logic
- Test when there are many callers of the same logic, and different callers
  depend on different behavior, making it easy to regress.
- Don't write tautological tests or tests for logic that won't regress.
  - Don't test a single match statement
  - Don't test that an unconditional print of a message prints the message
- **Tests: use test helpers** (behind `feature = "tests"`):
  - `flox_manifest::raw::test_helpers`: `mk_test_manifest_from_contents()`,
    `empty_test_migrated_manifest()`
  - `flox_manifest::test_helpers`: `with_latest_schema("body")` to prepend
    the correct schema version to TOML content strings

### Mock Data Generation

Mock catalog responses are generated against local floxhub services. See `CONTRIBUTING.md` for details on regenerating mocks.

## Activation scripts and hooks

Use [changing-activation-scripts](.agents/skills/changing-activation-scripts/SKILL.md)
when changing activation scripts, generated shell code, or hooks.

## Metrics / telemetry

All new CLI telemetry goes through the v2 events pipeline
(`cli/flox-events`); do not add to the legacy `subcommand_metric!`
stream. The wire contract — envelope shape, naming, stability and
privacy rules — is documented in `cli/flox-events/README.md`, and the
step-by-step procedure for adding an event or field is the
`adding-metrics-events` skill
(`.agents/skills/adding-metrics-events/SKILL.md`). Event and field names are frozen once
shipped: renames and type changes break downstream consumers silently.
A failing golden test in that crate is a contract signal — read the
README before touching the expected JSON.

## Pull Requests

Always follow `CONTRIBUTING.md`.

When opening a PR, consider if the PR has user-facing changes that aren't behind a feature flag.
If so, add a `## Release Notes` section to the PR description describing the change.
Most on-prem related features shouldn't include a release note,
so don't include one without confirmation.

Update your branch against `main` by rebasing — never a merge commit.

Hot fixes for a release target the release branch directly: open the PR
against `release-<version>` (e.g. `release-1.19.4`) rather than merging
into `main` and cherry-picking onto the release branch.

**Never mention customer names.** This is a public repository — keep
customer and prospect names out of everything that lands on GitHub:
PR titles and descriptions, commit messages, code comments, branch
names, test names, and test data. Refer to "a customer" and link the
internal Linear issue instead.

## CLI parsing

Use [bpaf](.agents/skills/bpaf/SKILL.md) when adding CLI subcommands or
changing command argument parsing with bpaf.

## Beta commands

For new beta commands, use
[adding-beta-subcommand](.agents/skills/adding-beta-subcommand/SKILL.md).

## Conventions

- **Rust style:**
  - Follow existing style and Rust idioms
  - Use early returns from functions and functional programming style; don't use nested conditionals
  - Structs should derive `Clone` and `Debug`
  - **Lifetimes: use sparingly.** Named lifetime parameters —
    especially on structs, where they propagate to every use of
    the type — are usually not worth the complexity. Prefer owned
    fields and prioritize readability over micro-optimizations
    like avoiding the clone of a short `&str`. Ordinary elided
    borrows in function signatures (`&str` parameters) are fine.
  - Use structured log and tracing fields; don't interpolate variables into single strings
  - `use` guidelines
    - Import from all flox crates with `use` rather than qualifying with `::`
    - For imports of external dependencies, qualifying with `::` is acceptable
      if it improves readability
    - Add `use` statements to modules; don't add to nearest function
    - Always update `use` statements when moving code between modules; don't re-export existing names
    - **DO NOT** import `catalog-api-v1` directly in `cli/flox/` (the main
      binary crate) or its tests. All catalog API types and errors must be
      accessed through `floxhub-client` or `flox-rust-sdk` re-exports.
      `catalog-api-v1` is generated code and its types are deliberately
      wrapped to keep generated-code churn inside one crate.
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
  - Use `formatdoc!` or `indoc!` (both from `indoc`) for multiline formatted
    strings. An exception is proc-macro attributes like (`#[error(...)]` and
    `#[bpaf(...)]`) which require string literals and cannot use
    macros.
  - **User-facing string literals:** Prefer stretching past the
    line-width limit rather than breaking messages with `\`
    continuations. The output the user sees matters more than
    source line length. Quote suggested commands with single
    quotes (e.g., `'git push'`).
  - **Type safety at function boundaries:** Parse strings
    at entry points (CLI arg parsing, API response
    deserialization), not deep in business logic. Before
    accepting a string parameter, check whether a domain type
    already exists: `Url` for URLs, `PackageSystem` for
    architecture names, `NixFlakeref` for flake refs,
    `BaseCatalogUrl` for catalog URLs. Check the relevant
    provider module for existing types, e.g. when working with
    catalog information check `cli/floxhub-client/src/types.rs`.
  - **Verify against the contract:** For generated API clients,
    confirm behavior (paging base, defaults, error shapes) against
    the OpenAPI spec or schema before relying on it. Do not write
    doc comments asserting behavior you have not verified.
  - **User-facing messages:** Use
    [changing-user-messages](.agents/skills/changing-user-messages/SKILL.md)
    when changing messages, CLI help, or man pages.
  - **Naming new helpers:** Before introducing a helper
    function, search for the naming convention used by similar
    helpers in the same file. Follow established patterns
    (`str_to_x` for query-param parsers in `floxhub-client`,
    `with_x` / `from_x` patterns elsewhere) rather than
    introducing a new convention.
  - **Deprecated infrastructure:** Before adding an
    implementation to an existing pattern, check for
    `// deprecated`, `// todo: remove`, or inline notes
    indicating the pattern is being phased out. When trait
    methods must be satisfied but the implementation is
    intentionally unused, use `unimplemented!()` and note
    it in the PR rather than adding to the deprecated pattern.
- **Commits:** Conventional commits format (`feat:`, `fix:`, `chore:`, etc.). Use `cz commit` for interactive commits
- **Rust 2024 edition** for main crates

## Manifest usage

Use [working-with-manifests](.agents/skills/working-with-manifests/SKILL.md)
when changing schemas, migration, or manifest read/write and lockfile paths.

## IDE Setup

See [CONTRIBUTING.md](CONTRIBUTING.md#setting-up-rust-ide-support) for rust-analyzer configuration.

## Comments

- Write concise comments
- Capture the "why" behind important decisions and explain those decisions, but
  don't just repeat what the code does.
- When explaining code, capture reasoning behind the final state of the
  code, but don't include the reasoning for individual commits.
  That belongs in a commit message.
  Don't narrate changes.
- A comment should document the code next to it. Don't explain another
  module's implementation details, and don't repeat the same explanation in
  multiple files — document a shared design decision once, in the most
  relevant place.
