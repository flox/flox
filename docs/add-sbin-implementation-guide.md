# How the `--add-sbin` change works, end to end

This document walks through every change made for "stop adding `sbin` to
`PATH` by default", in plain terms. It's written so that someone new to the
codebase can follow the whole path from the command line down to the shell
scripts, and knows how to test each piece.

## 1. The big picture

When you activate a Flox environment, Flox puts the environment's programs in
front of everything else by *prepending* directories to the `PATH` environment
variable. `PATH` is the colon-separated list of directories your shell
searches, left to right, when you type a command name.

Every environment has a `bin` directory (normal programs) and may have an
`sbin` directory (traditionally "system administration" programs). Until this
change, Flox prepended **both**:

```
/path/to/env/bin:/path/to/env/sbin:<rest of PATH>
```

That caused surprises: a package like BusyBox ships an `sbin/ifconfig` that
would shadow the `bin/ifconfig` of a dedicated networking package. So the new
default is **bin only**. Users opt back in per activation:

```bash
flox activate --add-sbin
```

The value is captured **per activation**: if you activate environment A with
the flag and environment B without it (Flox lets you "layer" activations
inside each other), only A's sbin lands on `PATH`.

## 2. How activation builds PATH (before this change)

There are several programs involved in one `flox activate`:

```
flox activate                       (Rust CLI, cli/flox)
   │  writes a JSON "context" file describing the activation
   ▼
flox-activations activate           (Rust helper binary, cli/flox-activations)
   │  computes environment variables, generates shell startup scripts
   ▼
your shell (bash/zsh/fish/tcsh)     sources those scripts, which call back into
   │                                two small helper subcommands:
   ├── flox-activations set-env-dirs   → maintains FLOX_ENV_DIRS
   └── flox-activations fix-paths      → rebuilds PATH and MANPATH
```

Two ideas matter here:

1. **`FLOX_ENV_DIRS`** is a colon-separated list of every currently-active
   environment, most recently activated first. It's the single source of
   truth for "what's active".
2. **PATH is rebuilt, not appended.** Your dotfiles (`.bashrc` etc.) can
   overwrite `PATH`, so Flox re-runs `fix-paths` *after* your dotfiles run.
   `fix-paths` takes `FLOX_ENV_DIRS` plus the current `PATH` and produces a
   repaired `PATH` with each env's directories at the front, deduplicated.

PATH is rebuilt in four different places, all funneling into the same Rust
function `fix_path_var()`:

1. In Rust, right before launching your shell
   (`fixed_vars_to_export()` in `cli/flox-activations/src/attach_diff/mod.rs`).
2. In the generated startup scripts for bash/fish/tcsh
   (`cli/flox-activations/src/gen_rc/{bash,fish,tcsh}.rs`), which run after
   your dotfiles.
3. In a static zsh script
   (`assets/environment-interpreter/activate/activate.d/zsh`).
4. In the interpreter's `activate` script's "command mode"
   (`assets/environment-interpreter/activate/activate`), used by package
   builds.

`fix_path_var()` used a hardcoded list of subdirectories: `["bin", "sbin"]`.
That one line was the old behavior.

## 3. The design: two new environment variables

Because PATH gets rebuilt over and over (possibly by a different process each
time), the "who wanted sbin?" information has to live somewhere every rebuild
can see it:

- **`_FLOX_ENV_DIRS_ADD_SBIN`** (exported) — the subset of `FLOX_ENV_DIRS`
  whose environments opted into sbin. Maintained exactly like `FLOX_ENV_DIRS`
  (prepend the env if it opted in and isn't already listed). `fix-paths` reads
  it and adds `<env>/sbin` right after `<env>/bin` *only* for envs in this
  list. This is the single durable piece of state.
- **`_FLOX_ADD_SBIN`** (transient, never exported) — `"true"`/`"false"` for
  the activation currently starting. The static shell scripts (which are the
  same file for every activation and can't have the value baked in) read this
  to know whether to pass `--add-sbin` to `set-env-dirs`, and it is unset
  immediately after use so it never lingers in the activated environment.

Worked example — nested activation, outer activation opted in, inner not:

```
FLOX_ENV_DIRS          = /inner:/outer
_FLOX_ENV_DIRS_ADD_SBIN = /outer
resulting PATH          = /inner/bin:/outer/bin:/outer/sbin:<rest>
```

## 4. Every change, by layer

### Layer 1: the core PATH logic (`cli/flox-activations`)

- `src/cli/fix_paths.rs` — `fix_path_var()` now takes a third argument, the
  sbin list. For each env dir it pushes `<dir>/bin`, and `<dir>/sbin` only if
  the dir is in the sbin set. The `fix-paths` subcommand gained a
  `--sbin-dirs` argument.
- `src/cli/set_env_dirs.rs` — the `set-env-dirs` subcommand gained
  `--sbin-dirs` (the current list) and `--add-sbin` (a flag). A new function
  `fix_sbin_dirs_var()` prepends the env to the list when the flag is set.
  The subcommand now emits *two* export lines: one for `FLOX_ENV_DIRS`, one
  for `_FLOX_ENV_DIRS_ADD_SBIN`.

### Layer 2: plumbing the flag through activation (`cli/flox-activations`)

- `cli/flox-core/src/activate/context.rs` — the `AttachCtx` struct (the JSON
  context the CLI hands to `flox-activations`) gained an `add_sbin: bool`
  field. It uses `#[serde(default)]` so old JSON files (e.g. baked into
  container images) still parse.
- `src/attach_diff/mod.rs` — `fixed_vars_to_export()` computes and exports the
  new sbin list (`_FLOX_ENV_DIRS_ADD_SBIN`); the in-place activation diff also
  tracks it, so `flox deactivate` restores it. `add_activate_script_options()`
  passes `--add-sbin` to the interpreter's `activate` script (mirroring how
  `--cuda-detection` already worked). `_FLOX_ADD_SBIN` is deliberately not
  exported here.
- `src/vars_from_env.rs` — reads `_FLOX_ENV_DIRS_ADD_SBIN` from the calling
  environment, alongside `FLOX_ENV_DIRS`/`PATH`/`MANPATH`.
- `src/gen_rc/{bash,fish,tcsh}.rs` — the generated startup scripts now pass
  `--sbin-dirs` (and `--add-sbin` when enabled) on their `set-env-dirs` and
  `fix-paths` lines. Each shell needed its own "default when unset" syntax
  (bash `${VAR:-}`, fish `set -q`, tcsh `$?VAR`).
- `src/gen_rc/zsh.rs` — zsh sources the static `activate.d/zsh` instead of
  baking the flag into a generated line, so its startup commands set
  `_FLOX_ADD_SBIN` for the current activation as a non-exported shell
  variable. Setting it is authoritative, not inherited: an in-place activation
  (`eval "$(flox activate)"`) has no spawned shell to receive the variable,
  and a value inherited from an outer activation must not leak into a nested
  one. `activate.d/zsh` unsets it after use.

### Layer 3: the flox CLI (`cli/flox`)

- `src/commands/activate.rs` — added the `--add-sbin` flag to
  `ActivateOptions`:

  ```rust
  let add_sbin = self.add_sbin;
  ```

  That value goes into `AttachCtx`.

The manifest and the global config deliberately have **no** say here: the
opt-in is per invocation. Keeping it out of the manifest also avoids a new
manifest schema version.

### Layer 4: the static shell scripts (`assets/environment-interpreter`)

- `activate/activate` — parses the new `--add-sbin` option into a plain (not
  exported) `_FLOX_ADD_SBIN` variable, and its command-mode block passes
  `--sbin-dirs` / `--add-sbin` to the helper subcommands.
- `activate/activate.d/zsh` — same treatment, keyed off the transient
  `_FLOX_ADD_SBIN` shell variable, which it unsets after use (zsh uses this
  static file instead of a generated script).

### Deliberately unchanged

- `assets/environment-interpreter/wrapper/wrapper` still puts `bin:sbin` on
  PATH — that's the wrapper for *built packages* (`flox build` results), a
  separate product surface. A follow-up can align it.
- `flox run`'s executable lookup still checks `bin/` then `sbin/`. That's an
  explicit lookup by name, not PATH shadowing, so the problem this change
  fixes doesn't apply there.

## 5. Try it yourself

All commands need the dev shell (`nix develop`, or prefix with
`nix develop -c`).

```bash
# Build everything
just build

# Default: no sbin on PATH
cd /tmp && mkdir sbin-demo && cd sbin-demo
/path/to/repo/target/debug/flox init
/path/to/repo/target/debug/flox activate -- sh -c 'echo $PATH'
# → .../run/<system>.sbin-demo.dev/bin:...   (no /sbin entry)

# Opt in with the flag
/path/to/repo/target/debug/flox activate --add-sbin -- sh -c 'echo $PATH'
# → .../bin:.../sbin:...
```

Run the relevant tests:

```bash
# Rust unit tests for the PATH logic
just unit-tests fix_paths
just unit-tests set_env_dirs

# Integration tests for activation PATH behavior (the sbin-specific ones
# are tagged activate:sbin)
just integ-tests activate.bats -- --filter-tags activate:sbin
just integ-tests activate.bats -- --filter 'patches PATH'
```
