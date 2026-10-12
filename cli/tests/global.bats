#!/usr/bin/env bats
# bats file_tags=global

load test_support.bash

setup_file() {
  # The inherited file teardown removes its cache. Give it a file-owned root
  # even though each test also uses a separate home and short cache directory.
  common_file_setup file
}

setup() {
  common_test_setup
  home_setup test
  GLOBAL_TEST_CACHE_ROOT="$FLOX_CACHE_DIR"
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
  mkdir -p "$BATS_TEST_TMPDIR/unrelated"
  cd "$(realpath "$BATS_TEST_TMPDIR/unrelated")" || return
  GLOBAL_ENV="$FLOX_DATA_DIR/environments/global"
  GLOBAL_STATE="$FLOX_DATA_DIR/global-publication/state.toml"
}

teardown() {
  if [[ -n "${GLOBAL_WORKER_PID:-}" ]]; then
    kill "$GLOBAL_WORKER_PID" 2>/dev/null || true
    wait "$GLOBAL_WORKER_PID" 2>/dev/null || true
  fi
  if [[ -n "${GLOBAL_ACTIVE_PROJECT:-}" ]]; then
    wait_for_activations "$GLOBAL_ACTIVE_PROJECT" || return 1
  fi
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then
    rm -rf "${GLOBAL_TEST_CACHE_ROOT:?}"
  fi
  common_test_teardown
}

hello_fixture() {
  HELLO_STORE="$("$NIX_BIN" build \
    --extra-experimental-features 'nix-command flakes' \
    "github:nixos/nixpkgs/$TEST_NIXPKGS_REV_NEW#hello^out" \
    --out-link "$BATS_TEST_TMPDIR/hello-fixture" --print-out-paths)"
}

install_hello() {
  hello_fixture
  "$FLOX_BIN" global install --id hello "$HELLO_STORE" --json
}

protected_state() {
  sha256sum "$GLOBAL_ENV/.flox/env/manifest.toml" \
    "$GLOBAL_ENV/.flox/env/manifest.lock" "$GLOBAL_STATE"
  readlink "$FLOX_DATA_DIR/bin/hello"
  readlink "$GLOBAL_ENV/.flox/run/$NIX_SYSTEM.global-run"
}

# bats test_tags=global:smoke
@test "global empty inventory and JSON usage errors do not initialize an environment" {
  run --separate-stderr "$FLOX_BIN" global list --json
  assert_success
  assert_equal "$(jq -r '.schema_version, (.after.packages | length)' <<<"$output")" $'1\n0'
  assert [ ! -e "$GLOBAL_ENV" ]
  run --separate-stderr "$FLOX_BIN" global dir --bin
  assert_success
  assert_output "$FLOX_DATA_DIR/bin"
  run --separate-stderr "$FLOX_BIN" global dir --sbin
  assert_success
  assert_output "$FLOX_DATA_DIR/sbin"
  run --separate-stderr "$FLOX_BIN" global uninstall --json
  assert_failure
  assert_equal "$(jq -r '.errors[0].category' <<<"$output")" usage
  assert [ ! -e "$GLOBAL_ENV" ]
}

# bats test_tags=global:smoke
@test "global native store-path lifecycle preserves ordinary run links" {
  run --separate-stderr install_hello
  assert_success
  assert_equal "$(jq -r '.environment_outcome, .publication.status' <<<"$output")" $'changed\ncomplete'
  local run_link
  run_link="$(realpath "$GLOBAL_ENV/.flox/run")/$NIX_SYSTEM.global-run"
  assert_equal "$(readlink "$FLOX_DATA_DIR/bin/hello")" "$run_link/bin/hello"
  run "$FLOX_DATA_DIR/bin/hello"
  assert_success
  assert_output 'Hello, world!'
  run --separate-stderr "$FLOX_BIN" global which hello --json
  assert_success
  assert_equal "$(jq -r '.after.commands[0].provider_install_ids[0]' <<<"$output")" hello
  run --separate-stderr "$FLOX_BIN" global upgrade --all --json
  assert_success
  run --separate-stderr "$FLOX_BIN" global uninstall hello --json
  assert_success
  assert [ ! -L "$FLOX_DATA_DIR/bin/hello" ]
}

# bats test_tags=global:smoke
@test "global repeat install retry and dry-run preserve authoritative state" {
  install_hello
  local before authority
  authority="$(sha256sum "$GLOBAL_ENV/.flox/env/manifest.toml" "$GLOBAL_ENV/.flox/env/manifest.lock")"
  before="$(protected_state)"
  run --separate-stderr "$FLOX_BIN" global install --id hello "$HELLO_STORE" --json
  assert_success
  assert_equal "$(jq -r '.environment_outcome' <<<"$output")" unchanged
  run --separate-stderr "$FLOX_BIN" global uninstall hello --dry-run --json
  assert_success
  assert_equal "$(jq -r '.dry_run, .publication.entries[0].action' <<<"$output")" $'true\nremove'
  assert_equal "$(protected_state)" "$before"
  run --separate-stderr "$FLOX_BIN" global retry-publication --dry-run --json
  assert_success
  assert_equal "$(protected_state)" "$before"
  # A missing recorded entry can be recreated from the current ordinary output.
  rm "$FLOX_DATA_DIR/bin/hello"
  # CLI startup needs a valid mock configuration; an empty mock cannot resolve
  # packages. Poison the actual build inputs only after preserving that setup.
  run --separate-stderr env NIX_BIN=/no-nix FLOX_BUILDENV_NIX=/no-build "$FLOX_BIN" global retry-publication --json
  assert_success
  assert_equal "$(sha256sum "$GLOBAL_ENV/.flox/env/manifest.toml" "$GLOBAL_ENV/.flox/env/manifest.lock")" "$authority"
  run "$FLOX_DATA_DIR/bin/hello"
  assert_success
  assert_output 'Hello, world!'
}

# bats test_tags=global:smoke
@test "global publication preserves foreign files and dangling links" {
  hello_fixture
  mkdir -p "$FLOX_DATA_DIR/bin"
  printf 'foreign\n' > "$FLOX_DATA_DIR/bin/hello"
  run --separate-stderr "$FLOX_BIN" global install --id hello "$HELLO_STORE" --json
  assert_failure
  assert_equal "$(jq -r '.environment_outcome, .publication.status' <<<"$output")" $'changed\nincomplete'
  assert_equal "$(cat "$FLOX_DATA_DIR/bin/hello")" foreign
  # Only the fixture moves its foreign entry aside; the publisher must not.
  mv "$FLOX_DATA_DIR/bin/hello" "$BATS_TEST_TMPDIR/foreign-file"
  ln -s /missing/foreign "$FLOX_DATA_DIR/bin/hello"
  run --separate-stderr "$FLOX_BIN" global retry-publication --json
  assert_failure
  assert_equal "$(readlink "$FLOX_DATA_DIR/bin/hello")" /missing/foreign
  assert_equal "$(jq -r '.publication.errors[0].category' <<<"$output")" ownership_conflict
}

# bats test_tags=global:smoke
@test "global printed PATH script works in a nested shell with the exact binary fixture" {
  install_hello
  local bash_bin script fixture_path
  bash_bin="$(command -v bash)"
  script="$BATS_TEST_TMPDIR/global-path.bash"
  fixture_path="$BATS_TEST_TMPDIR/exact-binary"
  mkdir -p "$fixture_path"
  ln -s "$FLOX_BIN" "$fixture_path/flox"
  "$FLOX_BIN" global update-shell --shell bash --print-script > "$script"
  run env PATH="$fixture_path" GLOBAL_SCRIPT="$script" \
    "$bash_bin" --noprofile --norc -c 'source "$GLOBAL_SCRIPT"; hello; flox global which hello --json'
  assert_success
  assert_line 'Hello, world!'
  local inventory
  inventory="${output#*$'\n'}"
  assert_equal "$(jq -r '.selected_by_path' <<<"$inventory")" "$FLOX_DATA_DIR/bin/hello"
}

@test "global shared suppliers and bin sbin entries preserve private runtime wrappers" {
  local packages first second run_link before
  packages="$("$NIX_BIN" build --impure --file "$TESTS_DIR/global/packages.nix" \
    --argstr rev "$TEST_NIXPKGS_REV_NEW" --argstr system "$NIX_SYSTEM" \
    first second --out-link "$BATS_TEST_TMPDIR/package-fixture" --print-out-paths)"
  first="$(head -n 1 <<<"$packages")"
  second="$(tail -n 1 <<<"$packages")"
  "$FLOX_BIN" global install --id first "$first" --json
  "$FLOX_BIN" global install --id second "$second" --json
  run_link="$(realpath "$GLOBAL_ENV/.flox/run")/$NIX_SYSTEM.global-run"
  assert_equal "$(readlink "$FLOX_DATA_DIR/bin/first")" "$run_link/bin/first"
  assert_equal "$(readlink "$FLOX_DATA_DIR/sbin/first")" "$run_link/sbin/first"
  assert [ ! -e "$FLOX_DATA_DIR/bin/private-helper" ]
  # Execution uses only the public wrapper and its private store references.
  run env -i PATH=/no-programs http_proxy=http://127.0.0.1:1 \
    "$FLOX_DATA_DIR/bin/first"
  assert_success
  assert_output 'private runtime'
  run --separate-stderr "$FLOX_BIN" global which shared --json
  assert_success
  assert_equal "$(jq -c '.after.commands[0].provider_install_ids' <<<"$output")" '["first","second"]'
  before="$(readlink "$FLOX_DATA_DIR/bin/shared")"
  # A command with multiple requesters is not an unambiguous uninstall target.
  run --separate-stderr "$FLOX_BIN" global uninstall shared --json
  assert_failure
  "$FLOX_BIN" global uninstall first --json
  assert [ ! -L "$FLOX_DATA_DIR/bin/first" ]
  assert [ ! -L "$FLOX_DATA_DIR/sbin/first" ]
  assert_equal "$(readlink "$FLOX_DATA_DIR/bin/shared")" "$before"
  run "$FLOX_DATA_DIR/bin/shared"
  assert_success
  assert_output shared
  run --separate-stderr "$FLOX_BIN" global which shared --json
  assert_success
  assert_equal "$(jq -c '.after.commands[0].provider_install_ids' <<<"$output")" '["second"]'
  # The ordinary run link remains the sole publication dependency/root path.
  assert [ -L "$run_link" ]
  assert_equal "$(readlink "$FLOX_DATA_DIR/bin/second")" "$run_link/bin/second"
}

@test "global invalid publication state blocks package and publication mutation" {
  install_hello
  local authority link state
  authority="$(sha256sum "$GLOBAL_ENV/.flox/env/manifest.toml" "$GLOBAL_ENV/.flox/env/manifest.lock")"
  link="$(readlink "$FLOX_DATA_DIR/bin/hello")"
  for state in 'not toml' 'version = 2' $'version = 1\nunknown = true'; do
    printf '%s\n' "$state" > "$GLOBAL_STATE"
    run --separate-stderr "$FLOX_BIN" global retry-publication --json
    assert_failure
    assert_equal "$(jq -r '.publication.errors[0].category' <<<"$output")" invalid_state
    run --separate-stderr "$FLOX_BIN" global uninstall hello --json
    assert_failure
    assert_equal "$(jq -r '.errors[0].category' <<<"$output")" invalid_state
    assert_equal "$(cat "$GLOBAL_STATE")" "$state"
    assert_equal "$(readlink "$FLOX_DATA_DIR/bin/hello")" "$link"
    assert_equal "$(sha256sum "$GLOBAL_ENV/.flox/env/manifest.toml" "$GLOBAL_ENV/.flox/env/manifest.lock")" "$authority"
  done
}

@test "global missing receipt does not adopt an existing link" {
  install_hello
  local link
  link="$(readlink "$FLOX_DATA_DIR/bin/hello")"
  mv "$GLOBAL_STATE" "$BATS_TEST_TMPDIR/saved-state.toml"
  run --separate-stderr "$FLOX_BIN" global retry-publication --json
  assert_failure
  assert_equal "$(jq -r '.publication.errors[0].category' <<<"$output")" ownership_conflict
  assert_equal "$(readlink "$FLOX_DATA_DIR/bin/hello")" "$link"
  assert [ ! -e "$GLOBAL_STATE" ]
}

@test "global unavailable ordinary output preserves packages receipts and public links" {
  install_hello
  local run_link before
  run_link="$GLOBAL_ENV/.flox/run/$NIX_SYSTEM.global-run"
  before="$(sha256sum "$GLOBAL_STATE")"
  mv "$run_link" "$BATS_TEST_TMPDIR/saved-run"
  run --separate-stderr env NIX_BIN=/no-nix FLOX_BUILDENV_NIX=/no-build \
    "$FLOX_BIN" global retry-publication --json
  assert_failure
  assert_equal "$(jq -r '.after.packages[0].install_id' <<<"$output")" hello
  assert_equal "$(sha256sum "$GLOBAL_STATE")" "$before"
  assert [ -L "$FLOX_DATA_DIR/bin/hello" ]
  mv "$BATS_TEST_TMPDIR/saved-run" "$run_link"
  run --separate-stderr env NIX_BIN=/no-nix FLOX_BUILDENV_NIX=/no-build \
    "$FLOX_BIN" global retry-publication --json
  assert_success
  run "$FLOX_DATA_DIR/bin/hello"
  assert_success
}

@test "global printed scripts preserve external precedence in every supported shell" {
  install_hello
  local shell shell_bin script fixture_path external_path
  fixture_path="$BATS_TEST_TMPDIR/exact-binary"
  external_path="$BATS_TEST_TMPDIR/external"
  mkdir -p "$fixture_path" "$external_path"
  ln -s "$FLOX_BIN" "$fixture_path/flox"
  printf '#!%s\necho external\n' "$(command -v bash)" > "$external_path/hello"
  chmod +x "$external_path/hello"
  for shell in bash zsh fish tcsh; do
    shell_bin="$(command -v "$shell")"
    script="$BATS_TEST_TMPDIR/global-path.$shell"
    "$FLOX_BIN" global update-shell --shell "$shell" --print-script > "$script"
    run --separate-stderr env PATH="$external_path:$fixture_path:$FLOX_DATA_DIR/bin:$FLOX_DATA_DIR/bin" \
      GLOBAL_SCRIPT="$script" FLOX_ENV=/sentinel-project GLOBAL_UNRELATED=unchanged \
      "$shell_bin" -c 'source "$GLOBAL_SCRIPT"; source "$GLOBAL_SCRIPT"; hello; flox global which hello --json'
    assert_success
    assert_line external
    assert_equal "$(jq -r '.selected_by_path' <<<"${output#*$'\n'}")" "$external_path/hello"
  done
}

@test "global shell setup is explicit idempotent and does not activate" {
  local shell before
  for shell in bash zsh fish tcsh; do
    run --separate-stderr "$FLOX_BIN" global update-shell --shell "$shell" --json
    assert_success
    assert [ "$(jq '.shell.changed_files | length' <<<"$output")" -gt 0 ]
    before="$(find "$HOME" -type f -exec sha256sum {} + | sort)"
    run --separate-stderr "$FLOX_BIN" global update-shell --shell "$shell" --json
    assert_success
    assert_equal "$(jq -c '.shell.changed_files' <<<"$output")" '[]'
    assert_equal "$(find "$HOME" -type f -exec sha256sum {} + | sort)" "$before"
  done
  assert [ ! -e "$GLOBAL_ENV" ]
  assert [ ! -e "$GLOBAL_STATE" ]
}

@test "global configured startup files work in interactive and login shells" {
  install_hello
  local shell shell_bin bash_bin login argv0
  bash_bin="$(command -v bash)"
  for shell in bash zsh fish tcsh; do
    shell_bin="$(command -v "$shell")"
    "$FLOX_BIN" global update-shell --shell "$shell" --json
    for login in no yes; do
      argv0="$shell"
      if [[ "$login" == yes ]]; then argv0="-$shell"; fi
      # A leading '-' is the login-shell convention, including tcsh, whose
      # -l flag cannot be combined with its -c command mode. No PTY is needed.
      run --separate-stderr "$bash_bin" --noprofile --norc -c \
        'exec -a "$1" "$2" -ic '\''hello; "$FLOX_BIN" global which hello --json'\''' \
        global-startup "$argv0" "$shell_bin"
      assert_success
      assert_line 'Hello, world!'
      assert_equal "$(jq -r '.selected_by_path' <<<"${output#*$'\n'}")" "$FLOX_DATA_DIR/bin/hello"
    done
  done
}

@test "global install and retry do not run ordinary activation configuration" {
  install_hello
  local manifest marker
  manifest="$BATS_TEST_TMPDIR/activation-manifest.toml"
  marker="$BATS_TEST_TMPDIR/activation-marker"
  cat "$GLOBAL_ENV/.flox/env/manifest.toml" > "$manifest"
  cat >> "$manifest" <<EOF
[vars]
GLOBAL_ACTIVATION_ONLY = "activation-only"
[hook]
on-activate = "touch '$marker'"
[profile]
common = "touch '$marker'"
[services.global-marker]
command = "touch '$marker'"
EOF
  "$FLOX_BIN" edit --dir "$GLOBAL_ENV" --file "$manifest"
  run --separate-stderr "$FLOX_BIN" global install --id hello "$HELLO_STORE" --json
  assert_success
  rm "$FLOX_DATA_DIR/bin/hello"
  run --separate-stderr "$FLOX_BIN" global retry-publication --json
  assert_success
  run "$FLOX_DATA_DIR/bin/hello"
  assert_success
  assert [ ! -e "$marker" ]
  assert [ -z "${GLOBAL_ACTIVATION_ONLY:-}" ]
}

# bats test_tags=activate
@test "global PATH setup preserves an active project's command precedence" {
  install_hello
  local bash_bin script setup_script
  bash_bin="$(command -v bash)"
  GLOBAL_ACTIVE_PROJECT="$BATS_TEST_TMPDIR/project"
  mkdir -p "$GLOBAL_ACTIVE_PROJECT"
  "$FLOX_BIN" init --dir "$GLOBAL_ACTIVE_PROJECT"
  "$FLOX_BIN" install --dir "$GLOBAL_ACTIVE_PROJECT" --id hello "$HELLO_STORE"
  setup_script="$BATS_TEST_TMPDIR/global-path.bash"
  "$FLOX_BIN" global update-shell --shell bash --print-script > "$setup_script"
  script="$BATS_TEST_TMPDIR/check-project.bash"
  cat > "$script" <<'EOF'
source "$GLOBAL_SETUP_SCRIPT"
"$FLOX_BIN" global which hello --json
EOF
  run --separate-stderr env GLOBAL_SETUP_SCRIPT="$setup_script" FLOX_SHELL="$bash_bin" \
    "$FLOX_BIN" activate --dir "$GLOBAL_ACTIVE_PROJECT" -- "$bash_bin" --noprofile --norc "$script"
  assert_success
  assert_equal "$(jq -r '.selected_by_path' <<<"$output")" \
    "$(realpath "$GLOBAL_ACTIVE_PROJECT/.flox/run")/$NIX_SYSTEM.project-dev/bin/hello"
}
