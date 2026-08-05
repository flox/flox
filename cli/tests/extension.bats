#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests for the `flox extension` subcommand and the `flox <name>` two-phase
# parse fallback that dispatches to a `flox-<name>` external executable.
#
# Covers dispatch and the local install / list / remove lifecycle.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=extension

# ---------------------------------------------------------------------------- #

setup() {
  # Skipped in setup so it covers every test in the file. Extensions are
  # gated behind `features.beta` and the subsystem lives in the lightly
  # reviewed `beta` module, so these don't run in CI. Delete this line
  # while working on them; the suite is expected to pass.
  skip "skipping tests for beta command, un-skip when hacking on beta commands"

  common_test_setup
  setup_isolated_flox
  # setup_isolated_flox exports FLOX_DATA_DIR, which the config system
  # resolves into `flox.data_dir`; dispatch and the subcommands both derive
  # the extensions root from it, so fixtures live under FLOX_DATA_DIR.
  export EXT_ROOT="${FLOX_DATA_DIR?}/extensions"
  mkdir -p "$EXT_ROOT"
  # Extensions are a beta feature and off by default. Enable for every test
  # here; the two tests that assert the disabled behavior `unset` it in
  # their own body, which runs after setup.
  export FLOX_FEATURES_BETA=true
}

teardown() {
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

@test "extension: 'flox <name>' dispatches to flox-<name> when extensions enabled" {
  export FLOX_FEATURES_BETA=true
  local ext_dir="$EXT_ROOT/flox-hello"
  mkdir -p "$ext_dir"
  cat > "$ext_dir/flox-hello" <<'EOF'
#!/usr/bin/env bash
echo "hello from extension"
EOF
  chmod +x "$ext_dir/flox-hello"

  run "$FLOX_BIN" hello
  assert_success
  assert_output --partial "hello from extension"
}

@test "extension: 'flox <name>' dispatch is off unless FLOX_FEATURES_BETA is set" {
  unset FLOX_FEATURES_BETA
  local ext_dir="$EXT_ROOT/flox-hello"
  mkdir -p "$ext_dir"
  cat > "$ext_dir/flox-hello" <<'EOF'
#!/usr/bin/env bash
echo "hello from extension"
EOF
  chmod +x "$ext_dir/flox-hello"

  run "$FLOX_BIN" hello
  assert_failure
  refute_output --partial "hello from extension"
}

# The name is the first argument that isn't a flag, so a global option's
# value has to be skipped along with the option itself.
@test "extension: dispatch looks past the value of a global option" {
  export FLOX_FEATURES_BETA=true
  _mk_managed_ext "hello" 'echo "hello from extension"'

  run "$FLOX_BIN" --floxhub-url https://example.com hello
  assert_success
  assert_output --partial "hello from extension"
}

@test "extension: 'flox extension --help' lists install/list/remove" {
  run "$FLOX_BIN" extension --help
  assert_success
  assert_output --partial "install"
  assert_output --partial "list"
  assert_output --partial "remove"
}

@test "extension: 'flox --help' does not list extension (beta commands are hidden)" {
  run "$FLOX_BIN" --help
  assert_success
  # bats-assert `--regexp` matches the entire `$output` as a single
  # string, so `^` only anchors to the very start. Match the line by
  # requiring a newline (or string start) before the leading spaces.
  refute_output --regexp '(^|'$'\n'')[[:space:]]*extension[[:space:]]+Manage flox extensions'
}

@test "extension: subcommands refuse to run when beta is disabled" {
  unset FLOX_FEATURES_BETA
  run "$FLOX_BIN" extension list
  assert_failure
  assert_output --partial "flox config --set features.beta true"
}

# P02-TS07: full install -> list -> dispatch -> remove -> list-empty lifecycle
# against a local source.
@test "extension: install/list/dispatch/remove lifecycle (local source via --from-path)" {
  export FLOX_FEATURES_BETA=true

  # Author a tiny local extension at $BATS_TEST_TMPDIR/flox-hello.
  local src="$BATS_TEST_TMPDIR/flox-hello"
  mkdir -p "$src"
  cat > "$src/flox-hello" <<'EOF'
#!/usr/bin/env bash
echo "hello from extension"
EOF
  chmod +x "$src/flox-hello"

  # Install — name derived from source dirname (no manifest).
  run "$FLOX_BIN" extension install --from-path "$src"
  assert_success
  assert_output --partial "Installed flox-hello"

  # List — table shows the name and the source path it was installed from.
  run "$FLOX_BIN" extension list
  assert_success
  assert_output --partial "hello"
  assert_output --partial "$src"

  # Dispatch — `flox hello` should now resolve to the installed executable.
  run "$FLOX_BIN" hello
  assert_success
  assert_output --partial "hello from extension"

  # Remove.
  run "$FLOX_BIN" extension remove hello
  assert_success
  assert_output --partial "Removed flox-hello"

  # List again — empty.
  run "$FLOX_BIN" extension list
  assert_success
  assert_output --partial "No extensions installed."
}

@test "extension: second install without --force emits the §2.9 message" {
  export FLOX_FEATURES_BETA=true
  local src="$BATS_TEST_TMPDIR/flox-hello"
  mkdir -p "$src"
  echo '#!/bin/sh' > "$src/flox-hello"
  chmod +x "$src/flox-hello"

  run "$FLOX_BIN" extension install --from-path "$src"
  assert_success

  run "$FLOX_BIN" extension install --from-path "$src"
  assert_failure
  assert_output --partial "flox-hello is already installed (run with --force to overwrite)"
}

@test "extension: --force install overwrites prior install" {
  export FLOX_FEATURES_BETA=true
  local src="$BATS_TEST_TMPDIR/flox-hello"
  mkdir -p "$src"
  cat > "$src/flox-hello" <<'EOF'
#!/usr/bin/env bash
echo "first version"
EOF
  chmod +x "$src/flox-hello"

  run "$FLOX_BIN" extension install --from-path "$src"
  assert_success

  # Change the source, reinstall with --force, and observe the new body.
  cat > "$src/flox-hello" <<'EOF'
#!/usr/bin/env bash
echo "second version"
EOF
  chmod +x "$src/flox-hello"

  run "$FLOX_BIN" extension install --from-path "$src" --force
  assert_success

  run "$FLOX_BIN" hello
  assert_success
  assert_output --partial "second version"
}

# P05-TS05: error strings from research-doc §2.9 must match verbatim.
@test "extension: install rejects reserved name with the §2.9 message" {
  export FLOX_FEATURES_BETA=true
  local src="$BATS_TEST_TMPDIR/flox-install"
  mkdir -p "$src"
  echo '#!/bin/sh' > "$src/flox-install"
  chmod +x "$src/flox-install"

  run "$FLOX_BIN" extension install --from-path "$src"
  assert_failure
  assert_output --partial "name 'install' conflicts with a built-in flox command"
}

# P11-TS05: drift guard for RESERVED_COMMAND_NAMES.
#
# `try_dispatch_external` only fires when bpaf fails to parse the first
# positional, so a built-in always shadows a same-named extension. The
# installer refuses reserved names to stop users installing something that
# could never dispatch — but that list is hand-maintained in
# `cli/flox/src/beta/extensions/reserved.rs` and silently rots when flox
# gains a command. This walks every visible top-level command and asserts the
# installer refuses it.
#
# Black-box on purpose: it exercises the shipped parser and the real
# installer, so it needs no test code in the `flox` crate.
#
# Parsing note: command rows are indented exactly four spaces. Options are
# also four-space indented but begin with `-`, and wrapped description text
# is indented far deeper — matching that by accident is how a previous
# version of this check mistook the wrapped word "invocation" for a
# command. Stopping at "Available options:" and requiring `[a-z]` after
# exactly four spaces excludes both.
#
# Hidden commands — the `hide`d `Commands` variants and everything under
# `InternalCommands` — never appear in --help and cannot be covered here;
# they are listed by hand in reserved.rs.
@test "extension: reserved-name list covers every visible top-level command" {
  export FLOX_FEATURES_BETA=true

  run "$FLOX_BIN" --help
  assert_success
  local commands
  commands="$(printf '%s\n' "$output" \
    | awk '/^Available options:/ {exit} /^    [a-z]/ {print $1}' \
    | tr -d ',' | sort -u)"

  # Guard the guard: if the help layout changes and we scrape nothing,
  # every assertion below would vacuously pass.
  local count
  count="$(printf '%s\n' "$commands" | grep -c .)"
  [ "$count" -ge 10 ] || {
    echo "expected >=10 top-level commands, parsed $count -- help layout changed?" >&2
    return 1
  }

  # `--from-path` derives the extension name from the *directory* basename,
  # which must be `flox-<name>`; give each candidate its own parent so the
  # directories don't collide.
  local cmd dir
  for cmd in $commands; do
    dir="$BATS_TEST_TMPDIR/reserved/$cmd/flox-$cmd"
    mkdir -p "$dir"
    echo '#!/bin/sh' > "$dir/flox-$cmd"
    chmod +x "$dir/flox-$cmd"

    run "$FLOX_BIN" extension install --from-path "$dir"
    assert_failure
    assert_output --partial "name '$cmd' conflicts with a built-in flox command"
  done
}

@test "extension: missing executable emits the §2.9 message" {
  export FLOX_FEATURES_BETA=true
  local src="$BATS_TEST_TMPDIR/flox-noexe"
  mkdir -p "$src"
  # Intentionally no flox-noexe executable inside.

  run "$FLOX_BIN" extension install --from-path "$src"
  assert_failure
  assert_output --partial "has no executable"
}

# ---------------------------------------------------------------------------- #
# Dispatch bookkeeping variables
#
# The managed-extension layout is `$EXT_ROOT/flox-<name>/flox-<name>`; dispatch
# reads `state.toml` from the same directory for `FLOX_EXTENSION_NAME`. This
# helper writes a managed extension with a user-supplied script body.
# ---------------------------------------------------------------------------- #

# Write a managed extension at $EXT_ROOT/flox-<name>/flox-<name>.
# Args: $1=name, $2=script_body (stdin body after shebang),
#       $3=state.toml name (optional, defaults to $1).
_mk_managed_ext() {
  local name="$1"
  local body="$2"
  local state_name="${3:-$1}"
  local ext_dir="$EXT_ROOT/flox-$name"
  mkdir -p "$ext_dir"
  printf '%s\n' '#!/usr/bin/env bash' "$body" > "$ext_dir/flox-$name"
  chmod +x "$ext_dir/flox-$name"
  # Minimal state.toml — supplies `FLOX_EXTENSION_NAME`.
  cat > "$ext_dir/state.toml" <<EOF
schema = "1"
name = "$state_name"
source = "."
installed_at = "1970-01-01T00:00:00Z"
path = "$ext_dir"
EOF
}

# The extension runs as a plain child process: it inherits the caller's
# environment untouched, and dispatch layers the FLOX_EXTENSION_* bookkeeping
# vars on top.
#
# The state.toml name deliberately differs from the dispatch token: with both
# spelled "probe" the `FLOX_EXTENSION_NAME` assertion would also pass if
# state.toml were never read, since dispatch falls back to the token.
# `FLOX_BIN` is set in the harness environment, so it's overridden with a
# sentinel to prove dispatch replaces it rather than the child inheriting it.
@test "extension: dispatch injects bookkeeping vars and inherits the caller env" {
  export FLOX_FEATURES_BETA=true
  _mk_managed_ext "probe" \
    'echo "FLOX_ENV=${FLOX_ENV:-unset}"
echo "EXT_NAME=${FLOX_EXTENSION_NAME:-unset}"
echo "EXT_PATH=${FLOX_EXTENSION_PATH:-unset}"
echo "EXT_FLOX_BIN=${FLOX_BIN:-unset}"' \
    "renamed"
  unset FLOX_ENV
  unset _FLOX_ACTIVE_ENVIRONMENTS

  run env FLOX_BIN=sentinel "$FLOX_BIN" probe
  assert_success
  assert_output --partial "FLOX_ENV=unset"
  assert_output --partial "EXT_NAME=renamed"
  assert_output --partial "EXT_PATH=$EXT_ROOT/flox-probe"
  refute_output --partial "EXT_FLOX_BIN=sentinel"
  assert_output --regexp 'EXT_FLOX_BIN=/.*flox'
}


# ---------------------------------------------------------------------------- #
# P07-TS01: docs pages present and relative links resolve
#
# Reduced scope replacement for the original docs-build check. No docs
# build tooling exists yet, so this asserts source presence and that
# relative `](./...)` / `](../...)` links resolve to real paths in the
# tree.
# ---------------------------------------------------------------------------- #

@test "extension: docs pages present and links resolve" {
  # The flox-cli-tests harness exports PROJECT_ROOT_DIR when running
  # against a real source checkout (the common `just integ-tests` path).
  # When the harness is launched from a /nix/store copy (the pure-Nix
  # `nix-integ-tests` path) there's no source tree to inspect, so the
  # presence check doesn't apply and we skip cleanly.
  if [ -z "${PROJECT_ROOT_DIR:-}" ]; then
    skip "PROJECT_ROOT_DIR not set (tests running from a Nix-built copy)"
  fi

  local docs="$PROJECT_ROOT_DIR/cli/flox/src/beta/extensions/docs"
  [ -f "$docs/README.md" ]
  [ -f "$docs/user-guide.md" ]
  [ -f "$docs/author-guide.md" ]

  # For each doc file, extract relative markdown links and assert
  # each target resolves against the file's directory. External
  # http(s) and anchor-only links are ignored.
  local f target abs rel
  for f in "$docs/README.md" "$docs/user-guide.md" "$docs/author-guide.md"; do
    while IFS= read -r target; do
      [ -z "$target" ] && continue
      # Strip any "#anchor" fragment.
      rel="${target%%#*}"
      [ -z "$rel" ] && continue
      abs="$(cd "$(dirname "$f")" && cd "$(dirname "$rel")" 2>/dev/null && pwd)/$(basename "$rel")" \
        || { echo "unresolved dir for link '$target' in $f" >&2; return 1; }
      if [ ! -e "$abs" ]; then
        echo "broken relative link '$target' in $f (resolved to $abs)" >&2
        return 1
      fi
    done < <(grep -oE '\]\((\./|\.\./)[^)]+\)' "$f" | sed -E 's/^\]\(//; s/\)$//')
  done
}

