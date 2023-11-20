#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox activate' subcommand.
# We are especially interested in ensuring that the activation script works
# with most common shells, since that routine will be executed using the users
# running shell.
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=activate


# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup;
}


# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}";
  export PROJECT_NAME="${PROJECT_DIR##*/}";
  rm -rf "$PROJECT_DIR";
  mkdir -p "$PROJECT_DIR";
  pushd "$PROJECT_DIR" >/dev/null||return;
  $FLOX_CLI init -d "$PROJECT_DIR";
}

project_teardown() {
  popd >/dev/null||return;
  rm -rf "${PROJECT_DIR?}";
  unset PROJECT_DIR;
  unset PROJECT_NAME;
}

activate_local_env() {
  run "$FLOX_CLI" activate -d "$PROJECT_DIR";
}


# ---------------------------------------------------------------------------- #

setup()    { common_test_setup; project_setup;       }
teardown() { project_teardown; common_test_teardown; }

# ---------------------------------------------------------------------------- #

activated_envs() {
  # Note that this variable is unset at the start of the test suite,
  # so it will only exist after activating an environment
  activated_envs=($(echo "$FLOX_PROMPT_ENVIRONMENTS"));
  echo "${activated_envs[*]}";
}

env_is_activated() {
  local is_activated;
  is_activated=0;
  for ae in $(activated_envs)
  do
    echo "activated_env = $ae, query = $1";
    if [[ "$ae" =~ "$1" ]]; then
      is_activated=1;
    fi
  done
  echo "$is_activated";
}

# ---------------------------------------------------------------------------- #

# `pkgdb lock` with no packages installed fetches a nixpkgs. With a package
# installed, it also has to evaluate the package set.
@test "warm up pkgdb" {
  run $FLOX_CLI install -d "$PROJECT_DIR" hello;
  assert_success;
  assert_output --partial "✅ 'hello' installed to environment";
  NIX_CONFIG="extra-experimental-features = flakes" "$PKGDB_BIN" manifest lock --ga-registry "$PROJECT_DIR/.flox/env/manifest.toml"
  # "$BUILD_ENV_BIN" "$NIX_BIN" "$NIX_SYSTEM" "$PROJECT_DIR/.flox/env/manifest.lock" "$PROJECT_DIR/.flox/run/$PROJECT_NAME.$NIX_SYSTEM" "$ENV_FROM_LOCKFILE_PATH";
}

# ---------------------------------------------------------------------------- #
@test "activate modifies prompt and puts package in path" {
  run $FLOX_CLI install -d "$PROJECT_DIR" hello;
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  SHELL=bash run expect -d "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR";
  assert_success;
}


# ---------------------------------------------------------------------------- #

@test "a1: 'flox develop' aliases to 'flox activate'" {
  skip FIXME;
  run "$FLOX_CLI" develop;
  assert_success;
  is_activated=$(env_is_activated "$PROJECT_NAME");
  assert_equal "$is_activated" "1";
}


# ---------------------------------------------------------------------------- #

@test "a2: activates environment in current dir by default" {
  skip FIXME;
  run "$FLOX_CLI" activate;
  assert_success;
  is_activated=$(env_is_activated "$PROJECT_NAME");
  assert_equal "$is_activated" "1";
}


# ---------------------------------------------------------------------------- #

@test "a3: 'flox activate' accepts explicit environment name" {
  skip FIXME;
  run "$FLOX_CLI" activate -d "$PROJECT_DIR"
  assert_success;
  is_activated=$(env_is_activated "$PROJECT_NAME");
  assert_equal "$is_activated" "1";
}


# ---------------------------------------------------------------------------- #

@test "a4: 'flox activate' modifies shell prompt with 'bash'" {
  skip FIXME;
  prompt_before="${PS1@P}";
  bash -c '"$FLOX_CLI" activate -d "$PROJECT_DIR"';
  assert_success;
  prompt_after="${PS1@P}";
  assert_not_equal prompt_before prompt_after;
  assert_regex prompt_after "flox \[.*$PROJECT_NAME.*\]"
}


# ---------------------------------------------------------------------------- #

@test "a4: 'flox activate' modifies shell prompt with 'zsh'" {
  skip FIXME;
  prompt_before="${(%%)PS1}";
  zsh -c '"$FLOX_CLI" activate -d "$PROJECT_DIR"';
  assert_success;
  prompt_after="${(%%)PS1}";
  assert_not_equal prompt_before prompt_after;
  assert_regex prompt_after "\[.*$PROJECT_NAME.*\]"
}


# ---------------------------------------------------------------------------- #

@test "a5: multiple activations are layered" {
  skip FIXME;
  # Steps
  # - Activate env1
  # - Activate env2
  # - Read activated envs with `activated_envs`
  # - Ensure that env2 (the last activated env) appears on the left
}


# ---------------------------------------------------------------------------- #

@test "a6: activate an environment by path" {
  skip FIXME;
  # Steps
  # - Activate an environment with the -d option
  # - Ensure that the environment is activated with `env_is_activated`
  is_activated=$(env_is_activated "$PROJECT_NAME");
  assert_equal "$is_activated" "1";
}


# ---------------------------------------------------------------------------- #

@test "a7: language specifics are set" {
  skip FIXME;
  # Steps
  # - Unset the PYTHON_PATH variable
  # - Install Python to the local environment
  # - Activate the environment
  # - Verify that PYTHON_PATH is set
}

# ---------------------------------------------------------------------------- #

@test "active environment is removed from active list after deactivating" {
  skip FIXME;
  # Steps
  # - Active an environment
  # - Verify that it appears in the list of active environments
  # - Exit the environment
  # - Ensure that it no longer appears in the list of active environments
}


# ---------------------------------------------------------------------------- #

@test "'flox activate' modifies path" {
  skip FIXME;
  original_path="$PATH";
  # Hangs because activate runs `nix shell` interactively right now
  run "$FLOX_CLI" activate -- echo "$PATH"
  assert_success;
  assert_equal "$original_path" "$output";
}
