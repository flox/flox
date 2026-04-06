# Profiling `flox activate`

## Quick Start

```bash
# Build with profiling enabled
nix develop -c cargo build --features profiling

# Also rebuild activation scripts (needed for shell-level tracing)
nix develop -c just build

# Run a profiled activate
FLOX_PROFILE=1 flox activate -c true

# Merge trace files into a single Perfetto-viewable timeline
python3 merge-profile-traces.py /tmp/flox-profile-<pid>/ -o /tmp/flox-profile-<pid>/merged.json

# Open merged.json at https://ui.perfetto.dev/
```

The profile directory path is printed to stderr when `FLOX_PROFILE=1` is set.

## What Gets Traced

### Rust CLI (`flox` binary)
- `cli_startup` -- logger, config, sentry, arg parsing
- `init_flox` -- directory setup, catalog client, Flox struct construction
- `activate::handle` / `activate::activate` -- the activate command

### Rust activations (`flox-activations` binary)
- `flox_activations_startup` -- post-logger initialization
- `activations::handle` -- activation entry point
- `activations::start` / `activations::attach` -- start vs attach path
- `spawn_executive` / `wait_for_executive` -- executive process lifecycle

### Shell activate script
- `activate` -- overall script
- `setup-env` -- combined profile.d + env setup
- `set_manifest_vars` -- sourcing envrc
- `fix-env` -- path deduplication (command mode only)

## Output Files

All files are written to `/tmp/flox-profile-<pid>/`:

| File | Format | Source |
|------|--------|--------|
| `flox-cli-<pid>.json` | Chrome Trace JSON | `flox` CLI binary |
| `flox-cli-<pid>.epoch` | Wall-clock microseconds | Epoch for time alignment |
| `flox-activations-<pid>.json` | Chrome Trace JSON | `flox-activations` binary |
| `flox-activations-<pid>.epoch` | Wall-clock microseconds | Epoch for time alignment |
| `shell-trace.ndjson` | Newline-delimited JSON | Bash activate script |

Multiple files may exist per type (e.g. the detached upgrade-check process writes its own `flox-cli-*.json`).

## Merge Tool

`merge-profile-traces.py` combines all trace sources into a single file:

```bash
python3 merge-profile-traces.py /tmp/flox-profile-<pid>/ [-o output.json]
```

The merge tool:
- Assigns each source file a unique process lane in Perfetto
- Aligns timestamps across processes using `.epoch` files
- Handles truncated JSON from processes that crashed or exec'd
- Labels each lane with the process name and original PID

## How It Works

- `FLOX_PROFILE=1` enables Chrome Trace output via the `tracing-chrome` crate (behind the `profiling` cargo feature flag)
- `FLOX_ACTIVATE_TRACE` is auto-enabled when `FLOX_PROFILE` is set, activating the shell-side tracer
- `_FLOX_PROFILE_DIR` is propagated to child processes so all traces land in the same directory
- `.epoch` files record the wall-clock time when each chrome layer was created, allowing the merge tool to align traces from different processes (since `tracing-chrome` uses process-local monotonic timestamps internally)
- `flush_chrome_trace()` is called before `exec()` calls to ensure trace data is written before the process is replaced

## Notes

- The `profiling` feature flag must be enabled at build time. Production builds without this feature have zero overhead from `#[instrument]` annotations (no active subscriber).
- `just build` does **not** enable the `profiling` feature. You need both `just build` (for activation scripts) and `cargo build --features profiling` (for Rust binaries).
- The `flox-activations` sub-commands (`fix-paths`, `set-env-dirs`, `prepend-and-dedup`) each create their own trace file, but they run so quickly the files are usually empty (`[]`).
