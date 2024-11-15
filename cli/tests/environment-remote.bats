#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox install`
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=remote

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="test"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return

}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  floxhub_setup owner
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown() {
  cat_teardown_fifo
  wait_for_watchdogs "$PROJECT_DIR"
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

function make_empty_remote_env() {
  NAME="${1:-test}"

  mkdir local
  pushd local
  # init path environment and push to remote
  "$FLOX_BIN" init --name "$NAME"
  "$FLOX_BIN" push --owner "$OWNER"
  "$FLOX_BIN" delete -f
  popd
  rm -rf local
}

# ---------------------------------------------------------------------------- #

# bats test_tags=hermetic,remote,remote:hermetic
@test "r0: listing a remote environment does not create (visible) local files" {
  make_empty_remote_env

  run --separate-stderr "$FLOX_BIN" list --name --remote "$OWNER/test"
  assert_success
  assert_output ""

  run ls -lA .
  assert_success
  assert_output "total 0"
}

# bats test_tags=hermetic,remote,remote:outlink
@test "r0: building a remote environment creates outlink" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  make_empty_remote_env

  run --separate-stderr "$FLOX_BIN" install hello --remote "$OWNER/test"
  assert_success

  assert [ -h "$FLOX_CACHE_DIR/run/$OWNER/$NIX_SYSTEM.test.dev" ]
  assert [ -h "$FLOX_CACHE_DIR/run/$OWNER/$NIX_SYSTEM.test.run" ]

}

# bats test_tags=install,remote,remote:install
@test "m1: install a package to a remote environment" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  make_empty_remote_env

  run "$FLOX_BIN" install hello --remote "$OWNER/test"
  assert_success
  assert_output --partial "environment '$OWNER/test' (remote)" # managed env output

  run --separate-stderr "$FLOX_BIN" list --name --remote "$OWNER/test"
  assert_success
  assert_output "hello"
}

# bats test_tags=uninstall,remote,remote:uninstall
@test "m2: uninstall a package from a remote environment" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/emacs_vim.json"
  make_empty_remote_env

  "$FLOX_BIN" install emacs vim --remote "$OWNER/test"

  run "$FLOX_BIN" uninstall vim --remote "$OWNER/test"
  assert_success

  run --separate-stderr "$FLOX_BIN" list --name --remote "$OWNER/test"
  assert_success
  assert_output "emacs"
}

# bats test_tags=edit,remote,remote:edit
@test "m3: edit a package from a managed environment" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  make_empty_remote_env

  TMP_MANIFEST_PATH="$BATS_TEST_TMPDIR/manifest.toml"

  cat <<"EOF" >>"$TMP_MANIFEST_PATH"
version = 1

[install]
hello.pkg-path = "hello"
EOF

  run "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH" --remote "$OWNER/test"
  assert_success
  assert_output --partial "✅ Environment successfully updated."

  run --separate-stderr "$FLOX_BIN" list --name --remote "$OWNER/test"
  assert_success
  assert_output "hello"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=remote,activate,remote:activate
@test "m9: activate works in remote environment" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  make_empty_remote_env
  "$FLOX_BIN" install hello --remote "$OWNER/test"

  export FLOX_CACHE_DIR="$(realpath $FLOX_CACHE_DIR)"
  run "$FLOX_BIN" activate --trust --remote "$OWNER/test" -- command -v hello
  assert_success
  assert_output --partial "$FLOX_CACHE_DIR/run/owner/$NIX_SYSTEM.test.dev/bin/hello"
}

# We need to trust the remote environment before we can activate it.
# bats test_tags=remote,activate,trust,remote:activate:trust-required
@test "m10.0: 'activate --remote' fails if remote environment is not trusted" {
  make_empty_remote_env

  run "$FLOX_BIN" activate --remote "$OWNER/test"
  assert_failure
  assert_output --partial "Environment $OWNER/test is not trusted."
}

# We can use the `--trust` flag to trust the environment temporarily.
# bats test_tags=remote,activate,trust,remote:activate:trust-option
@test "m10.1: 'activate --remote --trust' succeeds" {
  make_empty_remote_env

  run "$FLOX_BIN" activate --remote "$OWNER/test" --trust -- true
  assert_success
}

# We can use the `config to trust a specific remote environment.
# The `trust` flag is not required when activating a trusted environment.
# bats test_tags=remote,activate,trust,remote:activate:trust-config
@test "m10.2: 'activate --remote' succeeds if trusted by config" {
  make_empty_remote_env

  run "$FLOX_BIN" config --set "trusted_environments.'$OWNER/test'" "trust"
  run "$FLOX_BIN" activate --remote "$OWNER/test" -- true
  assert_success
}

# bats test_tags=remote,activate,trust,remote:activate:trust-config
@test "m10.2: 'activate --remote' succeeds if trusted by config (case-sensitive)" {
  make_empty_remote_env CaseSensitive

  run "$FLOX_BIN" config --set "trusted_environments.'$OWNER/CaseSensitive'" "trust"
  run "$FLOX_BIN" activate --remote "$OWNER/CaseSensitive" -- true
  assert_success
}

# We can use the `config to trust a specific remote environment.
# The `trust` flag is not required when activating a trusted environment.
# bats test_tags=remote,activate,trust,remote:activate:deny-config
@test "m10.3: 'activate --remote' fails if denied by config, --trust overrides" {
  make_empty_remote_env

  run "$FLOX_BIN" config --set "trusted_environments.'$OWNER/test'" "deny"

  run "$FLOX_BIN" activate --remote "$OWNER/test" -- true
  assert_failure

  run "$FLOX_BIN" activate --remote "$OWNER/test" --trust -- true
  assert_success
}

# bats test_tags=remote,activate,trust,remote:activate:trust-current-user
#
# If the remotely accessed environment is owned by the currently logged in user,
# we trust it automatically.
#
# flox reads the user handle from the auth token.
# The default floxhub test token has the user handle "test".
@test "m10.4: 'activate --remote' succeeds if owned by current user" {
  export OWNER="test"
  floxhub_setup "$OWNER"
  make_empty_remote_env

  run "$FLOX_BIN" activate --remote "$OWNER/test" -- true
  assert_success
}

# bats test_tags=remote,activate,trust,remote:activate:trust-flox
#
# If the remotely accessed environment is owned by Flox,
# we trust it automatically.
#
# flox reads the user handle from the auth token.
# Here we set a floxhub token with the user handle "test".
@test "m10.5: 'activate --remote' succeeds if owned by Flox" {
  floxhub_setup "flox"
  OWNER=flox make_empty_remote_env

  run "$FLOX_BIN" activate --remote "flox/test" -- true
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "sanity check upgrade works for remote environments" {
  skip "will be fixed by https://github.com/flox/flox/issues/1485"

  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    make_empty_remote_env

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.json" \
    "$FLOX_BIN" install hello --remote "$OWNER/test"

  run "$FLOX_BIN" list --remote "$OWNER/test"
  assert_success
  assert_output --partial "2.0"

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" upgrade --remote "$OWNER/test"

  run "$FLOX_BIN" list --remote "$OWNER/test"
  assert_success
  assert_output --partial "2.12.1"

  assert_output --partial "Upgraded 'hello'"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=remote,remote:not-found
@test "activate --remote fails on a non existent environment" {
  run "$FLOX_BIN" activate -r "$OWNER/i-dont-exist"
  assert_failure
  assert_output --partial "Environment not found in FloxHub."
}

# bats test_tags=remote,remote:not-found
@test "edit --remote fails on a non existent environment" {
  run "$FLOX_BIN" edit -r "$OWNER/i-dont-exist"
  assert_failure
  assert_output --partial "Environment not found in FloxHub."
}

# bats test_tags=remote,remote:not-found
@test "install --remote fails on a non existent environment" {
  run "$FLOX_BIN" install hello -r "$OWNER/i-dont-exist"
  assert_failure
  assert_output --partial "Environment not found in FloxHub."
}

# ---------------------------------------------------------------------------- #

# bats test_tags=remote,remote:auth-required,remote:auth-required:install
@test "'install --remote' fails if not authenticated" {
  unset FLOX_FLOXHUB_TOKEN # logout, effectively
  run "$FLOX_BIN" install hello --remote "$OWNER/test"
  assert_failure
  assert_output --partial "You are not logged in to FloxHub."
}

# bats test_tags=remote,remote:auth-required,remote:auth-required:uninstall
@test "'uninstall --remote' fails if not authenticated" {
  unset FLOX_FLOXHUB_TOKEN # logout, effectively
  run "$FLOX_BIN" uninstall hello --remote "$OWNER/test"
  assert_failure
  assert_output --partial "You are not logged in to FloxHub."
}

# bats test_tags=remote,remote:auth-required,remote:auth-required:edit
@test "'edit --remote' fails if not authenticated" {
  unset FLOX_FLOXHUB_TOKEN # logout, effectively
  run "$FLOX_BIN" edit --remote "$OWNER/test"
  assert_failure
  assert_output --partial "You are not logged in to FloxHub."
}

# bats test_tags=remote,remote:auth-required,remote:auth-required:upgrade
@test "'upgrade --remote' fails if not authenticated" {
  unset FLOX_FLOXHUB_TOKEN # logout, effectively
  run "$FLOX_BIN" upgrade hello --remote "$OWNER/test"
  assert_failure
  assert_output --partial "You are not logged in to FloxHub."
}

# bats test_tags=activate,activate:attach
@test "remote environments can attach" {
  project_setup
  export OWNER="owner"
  floxhub_setup "$OWNER"

  "$FLOX_BIN" init
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [hook]
    on-activate = """
      echo "sourcing hook.on-activate"
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -
  "$FLOX_BIN" push --owner "$OWNER"

  mkfifo started
  # Will get cat'ed in teardown
  TEARDOWN_FIFO="$PROJECT_DIR/finished"
  mkfifo "$TEARDOWN_FIFO"

  "$FLOX_BIN" activate --trust -r "$OWNER/test" -- bash -c "echo > started && echo > \"$TEARDOWN_FIFO\"" >> output 2>&1 &
  timeout 8 cat started
  run cat output
  assert_success
  assert_output --partial "sourcing hook.on-activate"


  run "$FLOX_BIN" activate --trust -r "$OWNER/test" -- true
  assert_success
  refute_output --partial "sourcing hook.on-activate"
}
