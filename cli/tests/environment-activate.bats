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

  export HELLO_PROFILE_SCRIPT=$(
    cat <<- EOF
[profile]
common = """
  echo "Welcome to your flox environment!";
"""
EOF
  )

  export VARS_PROFILE_SCRIPT=$(
    cat << EOF
[profile]
common = """
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
  FLOX_SHELL=bash USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/hello.exp" "$PROJECT_DIR"
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
  FLOX_SHELL=zsh USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# ---------------------------------------------------------------------------- #
# bats test_tags=activate,activate:hook:bash
@test "bash: activate runs 'profile.common'" {
  # calls init
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL=bash NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/hook.exp" "$PROJECT_DIR"
  assert_output --partial "Welcome to your flox environment!"

  FLOX_SHELL=bash USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "Welcome to your flox environment!"
}

# ---------------------------------------------------------------------------- #
# bats test_tags=activate,activate:hook:zsh
@test "zsh: activate runs 'profile.common'" {
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  # FLOX_SHELL=zsh USER="$REAL_USER" run -0 bash -c "echo exit | $FLOX_CLI activate --dir $PROJECT_DIR";
  FLOX_SHELL=zsh USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/hook.exp" "$PROJECT_DIR"
  assert_output --partial "Welcome to your flox environment!"

  FLOX_SHELL=zsh USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
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
  FLOX_SHELL=bash USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
  assert_output --partial "test_alias is aliased to \`echo testing'"
}

# ---------------------------------------------------------------------------- #
# bats test_tags=activate,activate:rc:zsh
@test "zsh: activate respects ~/.zshrc" {
  echo "alias test_alias='echo testing'" > "$HOME/.zshrc"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
  assert_output --partial "test_alias is an alias for echo testing"
}

# ---------------------------------------------------------------------------- #
# bats test_tags=activate,activate:envVar:bash
@test "bash: activate sets env var" {
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL=bash NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/envVar.exp" "$PROJECT_DIR"
  assert_output --partial "baz"

  FLOX_SHELL=bash NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- echo '$foo'
  assert_success
  assert_output --partial "baz"
}

# ---------------------------------------------------------------------------- #
# bats test_tags=activate,activate:envVar:zsh
@test "zsh: activate sets env var" {

  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL=zsh USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/envVar.exp" "$PROJECT_DIR"
  assert_output --partial "baz"

  FLOX_SHELL=zsh NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- echo '$foo'
  assert_success
  assert_output --partial "baz"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:envVar-before-hook:zsh
@test "zsh and bash: activate sets env var before hook" {
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[profile\]/${VARS_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL=zsh NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
  FLOX_SHELL=bash NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
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

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:inplace-prints
@test "'flox activate' prints script to modify current shell (bash)" {
  # Flox detects that the output is not a tty and prints the script to stdout
  #
  # TODO:
  # better with a flag like '--print-script'
  # this is confusing:
  FLOX_SHELL="bash" run "$FLOX_BIN" activate
  assert_success
  # check that env vars are set for compatibility with nix built software
  assert_line --partial "export NIX_SSL_CERT_FILE="
  assert_output --regexp "source .*/activate/bash"
}

# bats test_tags=activate,activate:inplace-prints
@test "'flox activate' prints script to modify current shell (zsh)" {
  FLOX_SHELL="zsh" run "$FLOX_BIN" activate
  assert_success
  # check that env vars are set for compatibility with nix built software
  assert_line --partial "export NIX_SSL_CERT_FILE="
  assert_output --regexp "source .*/activate/zsh"
}

# bats test_tags=activate,activate:inplace-modifies
@test "'flox activate' modifies the current shell (bash)" {

  # set a hook
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set vars
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  "$FLOX_BIN" install hello

  run bash -c 'eval "$("$FLOX_BIN" activate)"; type hello; echo $foo'
  assert_success
  # assert hook
  assert_line "Welcome to your flox environment!"
  # assert installed package
  assert_line --partial "hello is $(realpath $PROJECT_DIR)/.flox/run/"
  # assert var
  assert_line "baz"
}

# bats test_tags=activate,activate:inplace-modifies
@test "'flox activate' modifies the current shell (zsh)" {

  # set a hook
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set vars
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  "$FLOX_BIN" install hello

  run zsh -c 'eval "$("$FLOX_BIN" activate)"; type hello; echo $foo'
  assert_success
  assert_line "Welcome to your flox environment!"
  assert_line --partial "hello is $(realpath $PROJECT_DIR)/.flox/run/"
  assert_line "baz"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:inplace-reactivate
@test "bash: 'flox activate' only patches PATH when already activated" {
  SHELL="bash" run bash -c 'eval "$("$FLOX_BIN" activate --print-script)"; "$FLOX_BIN" activate --print-script'
  assert_success
  # on macos activating an already activated environment using
  # `eval "$(flox activate [--print-script])"
  # will only fix the PATH
  if [[ -e /usr/libexec/path_helper ]]; then
    assert_output --regexp "^(export PATH=.+)$"
  else
    # on linux reactivation is ignored
    assert_output ""
  fi
}

# bats test_tags=activate,activate:inplace-reactivate
@test "zsh: 'flox activate' only patches PATH when already activated" {
  SHELL="zsh" run zsh -c 'eval "$("$FLOX_BIN" activate --print-script)"; "$FLOX_BIN" activate --print-script'
  assert_success
  # on macos activating an already activated environment using
  # `eval "$(flox activate [--print-script])"
  # will only fix the PATH
  if [[ -e /usr/libexec/path_helper ]]; then
    assert_output --regexp "^(export PATH=.+)$"
  else
    # on linux reactivation is ignored
    assert_output ""
  fi
}

# bats test_tags=activate,activate:inplace-reactivate
@test "'flox activate' does not patch PATH when not activated" {
  run "$FLOX_BIN" activate --print-script
  assert_success
  refute_output --regexp "^(export PATH=.+)$"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:python-detects-installed-python
@test "'flox activate' sets python vars if python is installed" {
  # unset pyhton vars if any
  unset PYTHONPATH
  unset PIP_CONFIG_FILE

  # install python and pip
  "$FLOX_BIN" install python311Packages.pip

  run -- "$FLOX_BIN" activate -- echo PYTHONPATH is '$PYTHONPATH'
  assert_success
  assert_line "PYTHONPATH is $(realpath $PROJECT_DIR)/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/lib/python3.11/site-packages"

  run -- "$FLOX_BIN" activate -- echo PIP_CONFIG_FILE is '$PIP_CONFIG_FILE'
  assert_success
  assert_line "PIP_CONFIG_FILE is $(realpath $PROJECT_DIR)/.flox/pip.ini"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:python-retains-existing-python-vars
@test "'flox activate' retains existing python vars if python is not installed" {
  # set python vars
  export PYTHONPATH="/some/other/pythonpath"
  export PIP_CONFIG_FILE="/some/other/pip.ini"

  run -- "$FLOX_BIN" activate -- echo PYTHONPATH is '$PYTHONPATH'
  assert_success
  assert_line "PYTHONPATH is /some/other/pythonpath"

  run -- "$FLOX_BIN" activate -- echo PIP_CONFIG_FILE is '$PIP_CONFIG_FILE'
  assert_success
  assert_line "PIP_CONFIG_FILE is /some/other/pip.ini"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate:flox-uses-default-env
@test "'flox *' uses local environment over 'default' environment" {
  "$FLOX_BIN" delete

  mkdir default
  pushd default > /dev/null || return
  "$FLOX_BIN" init
  "$FLOX_BIN" install vim
  popd > /dev/null || return

  "$FLOX_BIN" init
  "$FLOX_BIN" install emacs

  # sanity check that flox list lists the local environment
  run -- "$FLOX_BIN" list -n
  assert_success
  assert_line "emacs"

  # Run flox list within the default environment.
  # Flox should choose the local environment over the default environment.
  run -- "$FLOX_BIN" activate --dir default -- "$FLOX_BIN" list -n
  assert_success
  assert_line "emacs"
}


# ---------------------------------------------------------------------------- #

# bats test_tags=activate:scripts:on-activate
@test "'hook.on-activate' runs" {
  "$FLOX_BIN" delete -f
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"
  # Run a command that causes the activation scripts to run without putting us
  # in the interactive shell
  run "$FLOX_BIN" activate -- echo "hello"
  # The on-activate script creates a directory whose name is the value of the
  # "$foo" environment variable.
  [ -d "$PROJECT_DIR/bar" ]
}


# ---------------------------------------------------------------------------- #

# bats test_tags=activate:scripts:on-activate
@test "'hook.on-activate' doesn't modify environment variables" {
  "$FLOX_BIN" delete -f
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"
  # Run a command that causes the activation scripts to run without putting us
  # in the interactive shell
  # What this is testing:
  # - Commands (e.g. echo "$foo") are run after activation scripts run
  # - The [vars] section sets foo=bar
  # - The on-activate script exports foo=baz
  # - If the on-activate script is able to modify variables outside the shell,
  #   then we should see "baz" here. The expected output is "bar" since that
  #   script isn't supposed to be able to modify environment variables.
  run "$FLOX_BIN" activate -- echo '$foo'
  assert_output "bar"
}


# ---------------------------------------------------------------------------- #

@test "bash: 'profile.common' is sourced before 'profile.bash'" {
  "$FLOX_BIN" delete -f
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  SHELL="bash" run bash -c '"$FLOX_BIN" activate -- true'
  # 'profile.common' sets a var containing "common",
  # 'profile.bash' creates a directory named after the contents of that
  # variable, suffixed by '-bash'
  [ -d "common-bash" ]
}


# ---------------------------------------------------------------------------- #

@test "zsh: 'profile.common' is sourced before 'profile.zsh'" {
  "$FLOX_BIN" delete -f
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  SHELL="zsh" run zsh -c '"$FLOX_BIN" activate -- true'
  # 'profile.common' sets a var containing "common",
  # 'profile.zsh' creates a directory named after the contents of that variable,
  # suffixed by '-zsh'
  [ -d "common-zsh" ]
}
