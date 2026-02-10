# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Flox is a virtual environment and package manager built on Nix. It creates portable, reproducible developer environments that can be shared across the software lifecycle.

**Languages:** Rust (CLI), Nix (packaging/environments), C++ (Nix plugins), Bash (activation scripts)

## Development Setup

```bash
nix develop                    # Enter dev shell with all dependencies
```

## Common Commands

```bash
# Building
just build                     # Build flox and all subsystems (debug)
just build-release             # Build optimized release version
just build-cli                 # Build only CLI (faster for Rust-only changes)

# Running
./cli/target/debug/flox --help # Run built binary
pushd cli; cargo run -- <args>; popd # Run via cargo

# Testing
just test-all                  # Full test suite (nix-plugins, unit, integration)
just test-cli                  # CLI tests only (impure + integration)
just unit-tests                # Unit tests
just impure-tests              # Unit tests with extra-tests feature
just integ-tests               # Integration tests (bats)
just unit-tests regex="test_name"     # Run specific unit test
just integ-tests usage.bats                       # Run specific integration test file
just integ-tests -- --filter-tags tag             # Run integration tests by tag
just integ-tests -- --filter regex                # Run integration tests by name
just integ-tests activate.bats -- --filter regex  # Run integration tests, filtering by both test file and test name. This is faster when wanting to run tests in a single file, because bats doesn't have to filter through all the tests in other files

# Formatting and Linting
just format                    # Format all code
pushd cli; cargo fmt; popd           # Format Rust
pushd cli; cargo clippy --all; popd  # Lint Rust
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
- **Commits:** Conventional commits format (`feat:`, `fix:`, `chore:`, etc.). Use `cz commit` for interactive commits
- **Rust 2024 edition** for main crates

## IDE Setup

For rust-analyzer, add to `.vscode/settings.json`:

```json
{
  "rust-analyzer.linkedProjects": ["${workspaceFolder}/cli/Cargo.toml"],
  "rust-analyzer.cargo.features": ["extra-tests"]
}
```
