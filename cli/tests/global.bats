#!/usr/bin/env bats
# bats file_tags=global

load test_support.bash

setup_file() {
  common_file_setup test
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
