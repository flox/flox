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
just integ-tests usage.bats           # Run specific test file
just integ-tests -- --filter-tags tag # Run tests by tag

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

### Integration Test Tags

Tests use bats tags for filtering: `init`, `build_env`, `install`, `uninstall`, `activate`, `push`, `pull`, `search`, `edit`, `list`, `delete`, `upgrade`, `project_env`, `managed_env`, `remote_env`, `python`, `node`, `go`, etc.

```bash
just integ-tests -- --filter-tags activate  # Run activation tests
```

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
  - Favor early returns from functions and functional programming style over nested conditionals-
  - Structs should derive `Clone` and `Debug`
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
