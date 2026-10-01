
#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test environment composition
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=compose

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
}

setup() {
  common_test_setup
  home_setup test # Isolate $HOME for each test.
  setup_isolated_flox
  project_setup

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown() {
  # fifo is in PROJECT_DIR and keeps executive running,
  # so cat_teardown_fifo must be run before wait_for_activations and
  # project_teardown
  cat_teardown_fifo
  # Cleaning up the `BATS_TEST_TMPDIR` occasionally fails,
  # because of an 'env-registry.json' that gets concurrently written
  # by the executive as the activation terminates.
  if [ -n "${PROJECT_DIR:-}" ]; then
    # Not all tests call project_setup
    wait_for_activations "$PROJECT_DIR" || return 1
    project_teardown
  fi
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return

}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

# ---------------------------------------------------------------------------- #
# Tests that share some helpers for setting up a composer and included
# environments
# ---------------------------------------------------------------------------- #

setup_composer_and_two_includes() {
  # Setup included1 environment
  "$FLOX_BIN" init -d included1
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    included1 = "v1"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d included1

  # Setup included2 environment
  "$FLOX_BIN" init -d included2
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    included2 = "v1"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d included2

  # Setup composer
  "$FLOX_BIN" init -d composer
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [include]
    environments = [
      { dir = "../included1" },
      { dir = "../included2" },
    ]
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d composer
}

# Modify vars.included1 in environment included1
edit_included1() {
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    included1 = "v2"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d included1
}

edit_both_included_environments() {
  edit_included1

  # Edit included2
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    included2 = "v2"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d included2

}

@test "include upgrade reports no changes" {
  setup_composer_and_two_includes
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/curl_three_systems_after_hello.yaml" \
    run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output --partial "No included environments have changes."
}

@test "include upgrade reports no changes when non-upgraded environment changes" {
  setup_composer_and_two_includes
  edit_included1
  run "$FLOX_BIN" include upgrade -d composer included2
  assert_success
  assert_output --partial "Included environment 'included2' has no changes."
}

@test "include upgrade defaults to upgrading all" {
  setup_composer_and_two_includes
  edit_both_included_environments

  run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output --partial - <<EOF
✔ Upgraded 'composer' with latest changes to:
- 'included1'
- 'included2'
EOF

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial 'included1 = "v2"'
  assert_output --partial 'included2 = "v2"'
}

@test "include upgrade can get latest changes for a single included environment" {
  setup_composer_and_two_includes
  edit_both_included_environments

  run "$FLOX_BIN" include upgrade -d composer included1
  assert_success
  assert_output --partial - <<EOF
✔ Upgraded 'composer' with latest changes to:
- 'included1'
EOF

  # Other commands use the latest changes to the include that wasn't named,
  # without saving them to the lockfile.
  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial - <<EOF
ℹ Using changes to included environments that aren't in the lockfile yet:
- 'included2'
Run 'flox include upgrade -d $(cd composer && pwd -P)' to save them to the lockfile.
EOF
  assert_output --partial 'included1 = "v2"'
  assert_output --partial 'included2 = "v2"'
}

@test "include upgrade reports which included environments have changes" {
  setup_composer_and_two_includes
  edit_included1

  run "$FLOX_BIN" include upgrade -d composer included1 included2
  assert_success
  assert_output --partial - <<EOF
✔ Upgraded 'composer' with latest changes to:
- 'included1'
ℹ Included environment 'included2' has no changes.
EOF

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial 'included1 = "v2"'
  assert_output --partial 'included2 = "v1"'
}

@test "include upgrade warns when implicit default systems change on re-lock" {
  if [ "$NIX_SYSTEM" == "x86_64-darwin" ]; then
    skip "implicit default systems are unchanged on x86_64-darwin"
  fi

  "$FLOX_BIN" init -d included
  cp "$GENERATED_DATA"/envs/hello_before_three_system_relock/manifest.{toml,lock} \
    included/.flox/env
  "$FLOX_BIN" init -d composer
  cp "$GENERATED_DATA"/envs/composer_before_three_system_relock/manifest.{toml,lock} \
    composer/.flox/env

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/curl_three_systems_after_hello.yaml" \
    "$FLOX_BIN" install -d included curl

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/include_upgrade_three_systems.yaml" \
    run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output --partial "! packages have been removed from lockfile for 'x86_64-darwin'"
  assert_output --partial "To reinstall, add 'x86_64-darwin' to 'options.systems' with 'flox edit'"
}

# ---------------------------------------------------------------------------- #
# Following included path environments
# ---------------------------------------------------------------------------- #

# The message that the composer is using changes to 'included1' that aren't
# saved to its lockfile
unsaved_included1() {
  cat <<EOF
ℹ Using changes to included environments that aren't in the lockfile yet:
- 'included1'
Run 'flox include upgrade -d $(cd composer && pwd -P)' to save them to the lockfile.
EOF
}

@test "activate follows changes to an included path environment without writing the lockfile" {
  setup_composer_and_two_includes
  edit_included1
  lockfile_before="$(cat composer/.flox/env/manifest.lock)"

  run --separate-stderr "$FLOX_BIN" activate -d composer -- bash -c 'echo "$included1"'
  assert_success
  assert_output "v2"
  assert_equal "$stderr" "$(unsaved_included1)"

  # The changes stay unsaved, so they're reported again.
  run --separate-stderr "$FLOX_BIN" activate -d composer -- bash -c 'echo "$included1"'
  assert_success
  assert_output "v2"
  assert_equal "$stderr" "$(unsaved_included1)"

  assert_equal "$(cat composer/.flox/env/manifest.lock)" "$lockfile_before"

  wait_for_activations "$PROJECT_DIR/composer" || return 1
}

@test "include upgrade saves the changes to an included path environment that are in use" {
  setup_composer_and_two_includes
  edit_included1

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial "$(unsaved_included1)"

  run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output --partial - <<EOF
✔ Upgraded 'composer' with latest changes to:
- 'included1'
EOF

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial 'included1 = "v2"'
  refute_output --partial "aren't in the lockfile yet"
}

@test "list follows locked changes to nested included path environments without writing to them" {
  "$FLOX_BIN" init -d c
  "$FLOX_BIN" init -d b
  "$FLOX_BIN" init -d a
  cat > c/.flox/env/manifest.toml << EOF
version = 1
[vars]
c = "v1"
EOF
  cat > b/.flox/env/manifest.toml << EOF
version = 1
[include]
environments = [{ dir = "../c" }]
EOF
  cat > a/.flox/env/manifest.toml << EOF
version = 1
[include]
environments = [{ dir = "../b" }]
EOF
  # Lock like 'flox edit' does, but without building
  "$FLOX_BIN" list -d c
  "$FLOX_BIN" list -d b

  run "$FLOX_BIN" list -c -d a
  assert_success
  assert_output --partial 'c = "v1"'

  lockfiles_before="$(cat a/.flox/env/manifest.lock b/.flox/env/manifest.lock)"
  sed -i -e 's/v1/v2/' c/.flox/env/manifest.toml
  "$FLOX_BIN" list -d c

  run "$FLOX_BIN" list -c -d a
  assert_success
  assert_output --partial - <<EOF
ℹ Using changes to included environments that aren't in the lockfile yet:
- 'b'
EOF
  assert_output --partial 'c = "v2"'

  assert_equal "$(cat a/.flox/env/manifest.lock b/.flox/env/manifest.lock)" "$lockfiles_before"
}

@test "list does not follow changes that an included path environment hasn't locked" {
  setup_composer_and_two_includes
  cat > included1/.flox/env/manifest.toml << EOF
version = 1
[vars]
included1 = "v2"
EOF

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial - <<EOF
! Could not get the latest changes to included environment 'included1'.
Using the version of 'included1' saved in the lockfile.
EOF
  assert_output --partial "has changes that aren't locked yet."
  assert_output --partial "Run 'flox edit -d"
  assert_output --partial 'included1 = "v1"'
}

@test "list does not follow changes to an included path environment with auto-upgrade disabled" {
  setup_composer_and_two_includes
  MANIFEST_CONTENTS="$(cat << "EOF"
    schema-version = "1.18.0"

    [include]
    environments = [
      { dir = "../included1", auto-upgrade = false },
      { dir = "../included2" },
    ]
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d composer
  edit_both_included_environments

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial 'included1 = "v1"'
  assert_output --partial 'included2 = "v2"'

  run "$FLOX_BIN" include upgrade -d composer included1
  assert_success

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial 'included1 = "v2"'
}

@test "list keeps the version in use of an included path environment it cannot read" {
  setup_composer_and_two_includes
  edit_included1
  "$FLOX_BIN" list -d composer
  mv included1 included1.moved

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial - <<EOF
! Could not get the latest changes to included environment 'included1'.
Using the version of 'included1' that was in use before.
EOF
  assert_output --partial 'included1 = "v2"'
  # Saving the others would lose the version of included1 in use
  assert_output --partial "to save them to the lockfile once all of them can be read."
}

@test "activate uses the lockfile when changes to an included path environment don't build" {
  "$FLOX_BIN" init -d included
  "$FLOX_BIN" edit -d included -f - << EOF
version = 1
[vars]
Y = "v1"
EOF
  "$FLOX_BIN" init -d composer
  "$FLOX_BIN" edit -d composer -f - << EOF
version = 1
[vars]
X = "\$Y"
[include]
environments = [{ dir = "../included" }]
EOF
  # Builds on its own, but the variables form a cycle with the composer's
  "$FLOX_BIN" edit -d included -f - << EOF
version = 1
[vars]
Y = "\$X"
EOF

  run --separate-stderr "$FLOX_BIN" activate -d composer -- bash -c 'echo "$X $Y"'
  assert_success
  assert_output "v1 v1"
  assert_regex "$stderr" "Could not build with the latest changes to included environments:"
  assert_regex "$stderr" "Found a reference cycle in the '\[vars\]' section"

  wait_for_activations "$PROJECT_DIR/composer" || return 1
}

@test "include upgrade --check succeeds when the lockfile has the latest changes" {
  setup_composer_and_two_includes

  run "$FLOX_BIN" include upgrade --check -d composer
  assert_success
  assert_output --partial "The lockfile has the latest changes to the included environments that commands use."
}

@test "include upgrade --check fails on changes that aren't in the lockfile, without writing anything" {
  setup_composer_and_two_includes
  edit_included1
  lockfile_before="$(cat composer/.flox/env/manifest.lock)"

  RUST_BACKTRACE=0 run "$FLOX_BIN" include upgrade --check -d composer
  assert_failure
  assert_output --partial - << EOF
The lockfile doesn't have the latest changes to included environments.

Included environments have changes that aren't in the lockfile:
- 'included1'

Run 'flox include upgrade -d $(cd composer && pwd -P)' to save them to the lockfile.
EOF
  assert_equal "$(cat composer/.flox/env/manifest.lock)" "$lockfile_before"
  run ls composer/.flox/cache
  refute_output --partial "followed-includes"

  "$FLOX_BIN" include upgrade -d composer
  run "$FLOX_BIN" include upgrade --check -d composer
  assert_success
}

@test "include upgrade --check fails when an included environment isn't locked" {
  setup_composer_and_two_includes
  # A manifest committed without its lockfile
  cat > included1/.flox/env/manifest.toml << EOF
version = 1
[vars]
included1 = "v2"
EOF
  rm included1/.flox/env/manifest.lock

  RUST_BACKTRACE=0 run "$FLOX_BIN" include upgrade --check -d composer
  assert_failure
  assert_output --partial "Could not get the latest changes to included environment 'included1'."
  assert_output --partial "has changes that aren't locked yet."
  assert_output --partial "Then run 'flox include upgrade -d $(cd composer && pwd -P)' to save the latest changes to the lockfile."
}

@test "include upgrade --check fails when the latest changes don't build" {
  "$FLOX_BIN" init -d included
  "$FLOX_BIN" edit -d included -f - << EOF
version = 1
[vars]
Y = "v1"
EOF
  "$FLOX_BIN" init -d composer
  "$FLOX_BIN" edit -d composer -f - << EOF
version = 1
[vars]
X = "\$Y"
[include]
environments = [{ dir = "../included" }]
EOF
  # Builds on its own, but the variables form a cycle with the composer's
  "$FLOX_BIN" edit -d included -f - << EOF
version = 1
[vars]
Y = "\$X"
EOF

  RUST_BACKTRACE=0 run "$FLOX_BIN" include upgrade --check -d composer
  assert_failure
  assert_output --partial - << EOF
The environment doesn't build with the latest changes to included environments:
- 'included'
EOF
  assert_output --partial "Found a reference cycle in the '[vars]' section"
}

@test "edit errors when an environment includes itself" {
  "$FLOX_BIN" init -d composer
  RUST_BACKTRACE=0 run "$FLOX_BIN" edit -d composer -f - << EOF
version = 1
[include]
environments = [{ dir = "." }]
EOF
  assert_failure
  # The path is canonicalized, so it doesn't necessarily start with $PROJECT_DIR.
  assert_output --regexp "environment '[^']*/composer' includes itself"
}

@test "list does not follow changes to included remote environments" {
  setup_composer_with_remote_include
  edit_remote

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial 'remote = "v1"'
  refute_output --partial "aren't in the lockfile yet"
}

@test "an environment pulled from FloxHub follows an included path environment without creating a generation" {
  floxhub_setup owner
  "$FLOX_BIN" init -d included
  "$FLOX_BIN" edit -d included -f - << EOF
version = 1
[vars]
included = "v1"
EOF
  "$FLOX_BIN" init -d composer
  "$FLOX_BIN" push -d composer --owner "$OWNER"
  # Pushing an environment that includes a path environment isn't allowed
  "$FLOX_BIN" edit -d composer -f - << EOF
version = 1
[include]
environments = [{ dir = "../included" }]
EOF
  generations() {
    "$FLOX_BIN" generations list -d composer --json | jq 'length'
  }
  generations_before="$(generations)"

  "$FLOX_BIN" edit -d included -f - << EOF
version = 1
[vars]
included = "v2"
EOF

  run --separate-stderr "$FLOX_BIN" activate -d composer -- bash -c 'echo "$included"'
  assert_success
  assert_output "v2"
  assert_regex "$stderr" "- 'included'"
  assert_equal "$(generations)" "$generations_before"

  run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output --partial "- 'included'"
  assert_equal "$(generations)" "$((generations_before + 1))"

  wait_for_activations "$PROJECT_DIR/composer" || return 1
}

# ---------------------------------------------------------------------------- #

function setup_composer_with_remote_include() {
  floxhub_setup owner

  # Setup owner/remote environment
  "$FLOX_BIN" init -d remote
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    remote = "v1"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d remote
  "$FLOX_BIN" push -d remote --owner "$OWNER"
  rm -rf remote

  # Setup composer
  "$FLOX_BIN" init -d composer
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [include]
    environments = [
      { remote = "owner/remote" },
    ]
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d composer
}

function edit_remote() {
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    remote = "v2"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -r owner/remote
  "$FLOX_BIN" push -f -r owner/remote
}

@test "list follows changes to an included remote environment with auto-upgrade enabled" {
  setup_composer_with_remote_include
  MANIFEST_CONTENTS="$(cat << "EOF"
    schema-version = "1.18.0"

    [include]
    environments = [
      { remote = "owner/remote", auto-upgrade = true },
    ]
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d composer
  lockfile_before="$(cat composer/.flox/env/manifest.lock)"
  edit_remote

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial 'remote = "v2"'
  assert_output --partial - <<EOF
ℹ Using changes to included environments that aren't in the lockfile yet:
- 'remote'
EOF
  assert_equal "$(cat composer/.flox/env/manifest.lock)" "$lockfile_before"
}

@test "include upgrade reports no changes for remote environments" {
  setup_composer_with_remote_include
  run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output - <<EOF
! Using file://${FLOX_FLOXHUB_PATH} as the FloxHub git endpoint.
'\$_FLOX_FLOXHUB_GIT_URL' overrides the git endpoint and is intended for testing only.

ℹ No included environments have changes.
EOF
}

@test "include upgrade reports which remote environments have changes" {
  setup_composer_with_remote_include
  edit_remote
  run "$FLOX_BIN" include upgrade -d composer
  assert_success
  assert_output - <<EOF
! Using file://${FLOX_FLOXHUB_PATH} as the FloxHub git endpoint.
'\$_FLOX_FLOXHUB_GIT_URL' overrides the git endpoint and is intended for testing only.

✔ Upgraded 'composer' with latest changes to:
- 'remote' (generation 1 -> 2)
EOF

  run "$FLOX_BIN" list -c -d composer
  assert_success
  assert_output --partial - <<EOF
ℹ Included FloxHub environments:
- 'remote' at generation 2
EOF
}

# ---------------------------------------------------------------------------- #
# Reusing the packages that included environments locked
# ---------------------------------------------------------------------------- #

@test "composing a locked environment reuses its locked packages" {
  "$FLOX_BIN" init -d included
  cp "$GENERATED_DATA"/envs/hello/manifest.{toml,lock} included/.flox/env

  # The file's default empty mock fails any resolution, so the composer can
  # only lock by reusing the included environment's packages.
  "$FLOX_BIN" init -d composer
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [include]
    environments = [
      { dir = "../included" },
    ]
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f - -d composer

  run "$FLOX_BIN" list -d composer
  assert_success
  assert_output --partial "hello: hello (2.12.3)"

  locked_packages='[.packages[] | {install_id, system, derivation}] | sort_by(.system)'
  assert_equal \
    "$(jq -S "$locked_packages" composer/.flox/env/manifest.lock)" \
    "$(jq -S "$locked_packages" included/.flox/env/manifest.lock)"
}

# ---------------------------------------------------------------------------- #
