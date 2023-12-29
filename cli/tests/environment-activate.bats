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

load test_support.bash

# bats file_tags=activate

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
}

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"

  export VARS=$(
    cat << EOF
[vars]
foo = "baz"
EOF
  )

  export HELLO_HOOK=$(
    cat <<- EOF
[hook]
script = """
  echo "Welcome to your flox environment!";
"""
EOF
  )

  export VARS_HOOK=$(
    cat << EOF
[hook]
script = """
  echo \$foo;
"""
EOF
  )

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
  "$FLOX_BIN" init -d "$PROJECT_DIR"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

activate_local_env() {
  run "$FLOX_BIN" activate -d "$PROJECT_DIR"
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  project_setup
}
teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

activated_envs() {
  # Note that this variable is unset at the start of the test suite,
  # so it will only exist after activating an environment
  activated_envs=($(echo "$FLOX_PROMPT_ENVIRONMENTS"))
  echo "${activated_envs[*]}"
}

env_is_activated() {
  local is_activated
  is_activated=0
  for ae in $(activated_envs); do
    echo "activated_env = $ae, query = $1"
    if [[ $ae =~ $1 ]]; then
      is_activated=1
    fi
  done
  echo "$is_activated"
}

# ---------------------------------------------------------------------------- #

@test "bash: activate modifies prompt and puts package in path" {
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  SHELL=bash USER="$REAL_USER" NO_COLOR=1 run -0 expect -d "$TESTS_DIR/activate/hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# ---------------------------------------------------------------------------- #

@test "zsh: activate modifies prompt and puts package in path" {
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  SHELL=zsh USER="$REAL_USER" NO_COLOR=1 run -0 expect -d "$TESTS_DIR/activate/hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# ---------------------------------------------------------------------------- #
# bats test_tags=activate,activate:hook:bash
@test "bash: activate runs hook" {
  sed -i -e "s/\[hook\]/${HELLO_HOOK//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  SHELL=bash NO_COLOR=1 run -0 expect -d "$TESTS_DIR/activate/hook.exp" "$PROJECT_DIR"
  assert_output --partial "Welcome to your flox environment!"

  SHELL=bash USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "Welcome to your flox environment!"
}

# ---------------------------------------------------------------------------- #
# bats test_tags=activate,activate:hook:zsh
@test "zsh: activate runs hook" {
  sed -i -e "s/\[hook\]/${HELLO_HOOK//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  # SHELL=zsh USER="$REAL_USER" run -0 bash -c "echo exit | $FLOX_CLI activate --dir $PROJECT_DIR";
  SHELL=zsh USER="$REAL_USER" NO_COLOR=1 run -0 expect -d "$TESTS_DIR/activate/hook.exp" "$PROJECT_DIR"
  assert_output --partial "Welcome to your flox environment!"

  SHELL=zsh USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "Welcome to your flox environment!"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:rc:bash
@test "bash: activate respects ~/.bashrc" {
  echo "alias test_alias='echo testing'" > "$HOME/.bashrc"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  SHELL=bash USER="$REAL_USER" NO_COLOR=1 run -0 expect -d "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
  assert_output --partial "test_alias is aliased to \`echo testing'"

  SHELL=bash USER="$REAL_USER" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- type test_alias
  assert_success
  assert_output --partial "test_alias is aliased to \`echo testing'"

}

# ---------------------------------------------------------------------------- #
# bats test_tags=activate,activate:rc:zsh
@test "zsh: activate respects ~/.zshrc" {
  echo "alias test_alias='echo testing'" > "$HOME/.zshrc"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect -d "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
  assert_output --partial "test_alias is an alias for echo testing"

  SHELL=zsh USER="$REAL_USER" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- type test_alias
  assert_success
  assert_output --partial "test_alias is an alias for echo testing"

}

# ---------------------------------------------------------------------------- #
# bats test_tags=activate,activate:envVar:bash
@test "bash: activate sets env var" {
  sed -i -e "s/\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  SHELL=bash NO_COLOR=1 run -0 expect -d "$TESTS_DIR/activate/envVar.exp" "$PROJECT_DIR"
  assert_output --partial "baz"

  SHELL=bash NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- echo '$foo'
  assert_success
  assert_output --partial "baz"
}

# ---------------------------------------------------------------------------- #
# bats test_tags=activate,activate:envVar:zsh
@test "zsh: activate sets env var" {

  sed -i -e "s/\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  SHELL=zsh USER="$REAL_USER" NO_COLOR=1 run -0 expect -d "$TESTS_DIR/activate/envVar.exp" "$PROJECT_DIR"
  assert_output --partial "baz"

  SHELL=zsh  NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- echo '$foo'
  assert_success
  assert_output --partial "baz"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:envVar-before-hook:zsh
@test "zsh and bash: activate sets env var before hook" {
  sed -i -e "s/\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/\[hook\]/${VARS_HOOK//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  SHELL=zsh NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
  SHELL=bash NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
}

# ---------------------------------------------------------------------------- #

@test "a1: 'flox develop' aliases to 'flox activate'" {
  skip FIXME
  run "$FLOX_BIN" develop
  assert_success
  is_activated=$(env_is_activated "$PROJECT_NAME")
  assert_equal "$is_activated" "1"
}

# ---------------------------------------------------------------------------- #

@test "a2: activates environment in current dir by default" {
  skip FIXME
  run "$FLOX_BIN" activate
  assert_success
  is_activated=$(env_is_activated "$PROJECT_NAME")
  assert_equal "$is_activated" "1"
}

# ---------------------------------------------------------------------------- #

@test "a3: 'flox activate' accepts explicit environment name" {
  skip FIXME
  run "$FLOX_BIN" activate -d "$PROJECT_DIR"
  assert_success
  is_activated=$(env_is_activated "$PROJECT_NAME")
  assert_equal "$is_activated" "1"
}

# ---------------------------------------------------------------------------- #

@test "a4: 'flox activate' modifies shell prompt with 'bash'" {
  skip FIXME
  prompt_before="${PS1@P}"
  bash -c '"$FLOX_BIN" activate -d "$PROJECT_DIR"'
  assert_success
  prompt_after="${PS1@P}"
  assert_not_equal prompt_before prompt_after
  assert_regex prompt_after "flox \[.*$PROJECT_NAME.*\]"
}

# ---------------------------------------------------------------------------- #

# Commented out until someone decides to make this test pass,
# otherwise shellcheck complains.
# @test "a4: 'flox activate' modifies shell prompt with 'zsh'" {
#   skip FIXME;
#   prompt_before="${(%%)PS1}";
#   zsh -c '"$FLOX_BIN" activate -d "$PROJECT_DIR"';
#   assert_success;
#   prompt_after="${(%%)PS1}";
#   assert_not_equal prompt_before prompt_after;
#   assert_regex prompt_after "\[.*$PROJECT_NAME.*\]"
# }

# ---------------------------------------------------------------------------- #

@test "a5: multiple activations are layered" {
  skip FIXME
  # Steps
  # - Activate env1
  # - Activate env2
  # - Read activated envs with `activated_envs`
  # - Ensure that env2 (the last activated env) appears on the left
}

# ---------------------------------------------------------------------------- #

@test "a6: activate an environment by path" {
  skip FIXME
  # Steps
  # - Activate an environment with the -d option
  # - Ensure that the environment is activated with `env_is_activated`
  is_activated=$(env_is_activated "$PROJECT_NAME")
  assert_equal "$is_activated" "1"
}

# ---------------------------------------------------------------------------- #

@test "a7: language specifics are set" {
  skip FIXME
  # Steps
  # - Unset the PYTHON_PATH variable
  # - Install Python to the local environment
  # - Activate the environment
  # - Verify that PYTHON_PATH is set
}

# ---------------------------------------------------------------------------- #

@test "active environment is removed from active list after deactivating" {
  skip FIXME
  # Steps
  # - Active an environment
  # - Verify that it appears in the list of active environments
  # - Exit the environment
  # - Ensure that it no longer appears in the list of active environments
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:path
@test "'flox activate' modifies path" {
  original_path="$PATH"
  run "$FLOX_BIN" activate -- echo '$PATH'
  assert_success
  assert_not_equal "$original_path" "$output"

  # hello is not on the path
  run -1 type hello

  run "$FLOX_BIN" install hello
  assert_success

  run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"
}
