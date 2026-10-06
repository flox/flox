#! /usr/bin/env bats
# -*- mode: bats; -*-
# Inspect emitted image bytes without a GPU, root, or a container runtime.
# macOS still needs the existing runtime-backed containerize.bats suite.

load test_support.bash

# bats file_tags=containerize

setup() {
  common_test_setup
  if [[ "$(uname -s)" != Linux ]]; then
    skip "Native image assembly requires Linux"
  fi
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project"
  mkdir -p "$PROJECT_DIR"
  "$FLOX_BIN" init -d "$PROJECT_DIR"
}

# Replace the init template so that the absent case really has no CUDA option.
set_image_manifest() {
  local setting="$1"
  local user="${2:-}"
  local working_dir="${3:-}"
  {
    with_latest_schema ""
    printf '\n[options]\n'
    if [[ "$setting" != absent ]]; then
      printf 'cuda-detection = %s\n' "$setting"
    fi
    if [[ -n "$user" ]]; then
      printf '\n[containerize.config]\nuser = "%s"\n' "$user"
      if [[ -n "$working_dir" ]]; then
        printf 'working-dir = "%s"\n' "$working_dir"
      fi
    fi
  } > "$BATS_TEST_TMPDIR/manifest.toml"
  "$FLOX_BIN" edit -d "$PROJECT_DIR" -f "$BATS_TEST_TMPDIR/manifest.toml"
}

assert_image_modes() {
  local cuda="$1"
  shift
  local mode image
  for mode in dev run; do
    image="$BATS_TEST_TMPDIR/image-$mode.tar"
    run "$FLOX_BIN" containerize -d "$PROJECT_DIR" --mode "$mode" --file "$image"
    assert_success
    run python3 "$TESTS_DIR/container/verify_image.py" "$image" --mode "$mode" --cuda "$cuda" "$@"
    assert_success
    rm "$image"
  done
}

@test "container image verifier rejects regressed archive payloads" {
  run python3 "$TESTS_DIR/container/test_verify_image.py"
  assert_success
}

@test "containerize enables CUDA detection when the manifest option is absent" {
  set_image_manifest absent
  assert_image_modes 1
}

@test "containerize honors explicit CUDA detection true" {
  set_image_manifest true
  assert_image_modes 1
}

@test "containerize honors explicit CUDA detection false" {
  set_image_manifest false
  assert_image_modes 0
}

@test "containerize preserves named user and group records and writable directories" {
  set_image_manifest absent foo:bar /workspace
  assert_image_modes 1 --user foo:bar --uid 10000 --gid 10000 --working-dir /workspace \
    --passwd-line 'foo:x:10000:10000:created by Flox:/var/empty:/bin/sh' \
    --group-line 'bar:x:10000:'
}

@test "containerize preserves numeric user and group records and writable directories" {
  set_image_manifest false 1234:5678 /workspace
  assert_image_modes 0 --user 1234:5678 --uid 1234 --gid 5678 --working-dir /workspace \
    --passwd-line 'flox:x:1234:5678:created by Flox:/var/empty:/bin/sh' \
    --group-line 'flox:x:5678:'
}

@test "containerize preserves an explicit root account without duplicate records" {
  set_image_manifest true root:root
  assert_image_modes 1 --user root:root
}

@test "containerize uses the composed CUDA option and the top-level override" {
  local included="$BATS_TEST_TMPDIR/included"
  mkdir -p "$included"
  "$FLOX_BIN" init -d "$included"
  {
    with_latest_schema ""
    printf '\n[options]\ncuda-detection = false\n'
  } > "$BATS_TEST_TMPDIR/included.toml"
  "$FLOX_BIN" edit -d "$included" -f "$BATS_TEST_TMPDIR/included.toml"
  {
    with_latest_schema ""
    printf '\n[include]\nenvironments = [{ dir = "%s" }]\n' "$included"
  } > "$BATS_TEST_TMPDIR/composed.toml"
  "$FLOX_BIN" edit -d "$PROJECT_DIR" -f "$BATS_TEST_TMPDIR/composed.toml"
  assert_image_modes 0

  printf '\n[options]\ncuda-detection = true\n' >> "$BATS_TEST_TMPDIR/composed.toml"
  "$FLOX_BIN" edit -d "$PROJECT_DIR" -f "$BATS_TEST_TMPDIR/composed.toml"
  assert_image_modes 1
}
