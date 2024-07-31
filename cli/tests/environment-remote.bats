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
  pushd "$PROJECT_DIR" > /dev/null || return

}

# tests should not share the same floxmeta repo.
# we also want to simulate different machines.
#
# floxmeta_setup <machine_name>
floxmeta_setup() {
  mkdir -p "$FLOXHUB_FLOXMETA_DIR/${1}"
  export FLOX_DATA_DIR="$BATS_TEST_TMPDIR/${1}"
}

project_teardown() {
  popd > /dev/null || return
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
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

function make_empty_remote_env() {
  mkdir local
  pushd local
  # init path environment and push to remote
  "$FLOX_BIN" init --name test
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
  export FLOX_FEATURES_USE_CATALOG=false
  make_empty_remote_env

  run --separate-stderr "$FLOX_BIN" install hello --remote "$OWNER/test"
  assert_success

  assert [ -h "$FLOX_CACHE_DIR/run/$OWNER/test" ]
}

# bats test_tags=hermetic,remote,remote:outlink
@test "catalog: r0: building a remote environment creates outlink" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  make_empty_remote_env

  run --separate-stderr "$FLOX_BIN" install hello --remote "$OWNER/test"
  assert_success

  assert [ -h "$FLOX_CACHE_DIR/run/$OWNER/test" ]
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
  export FLOX_FEATURES_USE_CATALOG=false
  make_empty_remote_env

  TMP_MANIFEST_PATH="$BATS_TEST_TMPDIR/manifest.toml"

  cat << "EOF" >> "$TMP_MANIFEST_PATH"
[install]
hello = {}
EOF

  run "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH" --remote "$OWNER/test"
  assert_success
  assert_output --partial "✅ Environment successfully updated."

  run --separate-stderr "$FLOX_BIN" list --name --remote "$OWNER/test"
  assert_success
  assert_output "hello"
}

# bats test_tags=edit,remote,remote:edit
@test "catalog: m3: edit a package from a managed environment" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  make_empty_remote_env

  TMP_MANIFEST_PATH="$BATS_TEST_TMPDIR/manifest.toml"

  cat << "EOF" >> "$TMP_MANIFEST_PATH"
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
  export FLOX_FEATURES_USE_CATALOG=false
  make_empty_remote_env
  "$FLOX_BIN" install hello --remote "$OWNER/test"

  run "$FLOX_BIN" activate --trust --remote "$OWNER/test" -- command -v hello
  assert_success
  assert_output --partial "$FLOX_CACHE_DIR/remote/owner/test/.flox/run/bin/hello"
}

# bats test_tags=remote,activate,remote:activate
@test "catalog: m9: activate works in remote environment" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  make_empty_remote_env
  "$FLOX_BIN" install hello --remote "$OWNER/test"

  run "$FLOX_BIN" activate --trust --remote "$OWNER/test" -- command -v hello
  assert_success
  assert_output --partial "$FLOX_CACHE_DIR/remote/owner/test/.flox/run/bin/hello"
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
  run "$FLOX_BIN" install -r "$OWNER/i-dont-exist"
  assert_failure
  assert_output --partial "Environment not found in FloxHub."
}

# ---------------------------------------------------------------------------- #

# bats test_tags=remote,remote:services
@test "services: not currently supported for remote environments" {
  export FLOX_FEATURES_SERVICES=true

  floxhub_setup "flox"
  OWNER=flox make_empty_remote_env

  TMP_MANIFEST_PATH="$BATS_TEST_TMPDIR/manifest.toml"

  cat << "EOF" >> "$TMP_MANIFEST_PATH"
version = 1

[services.hello]
command = "hello"
EOF

  run "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH" --remote "$OWNER/test"
  assert_success
  assert_output --partial "✅ Environment successfully updated."

  run "$FLOX_BIN" activate --remote "flox/test" -- true
  assert_failure
  assert_output --partial "❌ ERROR: services are not currently supported for remote environments"

  run "$FLOX_BIN" services stop hello --remote "flox/test"
  assert_failure
  assert_output --partial "❌ ERROR: services are not currently supported for remote environments"

  run "$FLOX_BIN" services logs hello --remote "flox/test"
  assert_failure
  assert_output --partial "❌ ERROR: services are not currently supported for remote environments"
}
