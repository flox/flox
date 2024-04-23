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
  echo "sourcing profile.common";
"""
bash = """
  echo "sourcing profile.bash";
"""
zsh = """
  echo "sourcing profile.zsh";
"""
EOF
  )

  export VARS_HOOK_SCRIPT=$(
    cat << EOF
[hook]
on-activate = """
  echo "sourcing hook.on-activate";
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
  setup_isolated_flox # concurrent pkgdb database creation
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

# bats test_tags=activate,activate:flox_shell,activate:flox_shell:bash
@test "activate identifies FLOX_SHELL from running shell (bash)" {
  run --separate-stderr bash -c "$FLOX_BIN activate | grep -- '-flox-activate.d/set-prompt'"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_line --partial "flox-activate.d/set-prompt.bash"
}

# bats test_tags=activate,activate:flox_shell,activate:flox_shell:zsh
@test "activate identifies FLOX_SHELL from running shell (zsh)" {
  run --separate-stderr zsh -c "$FLOX_BIN activate | grep -- '-flox-activate.d/set-prompt'"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_line --partial "flox-activate.d/set-prompt.zsh"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:path,activate:path:bash
@test "bash: activate puts package in path" {
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:zsh
@test "zsh: activate puts package in path" {
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# ---------------------------------------------------------------------------- #

# The following battery of tests ensure that the activation script invokes
# the expected hook and profile scripts for the bash and zsh shells, and
# in each of the following four scenarios:
#
# 1. in the interactive case, simulated using using `hook.exp`
# 2. in the default command case, invoking the shell primitive `:` (a no-op)
# 3. in the `--noprofile` command case, again invoking the shell primitive `:`
# 4. in the `--turbo` command case, which exec()s the provided command without
#    involving the userShell and instead invokes `true` from the PATH
#
# The question of whether to continue support for the --noprofile and --turbo
# cases is still open for discussion, but the tests are included here to ensure
# that the current behavior is consistent and predictable.

# bats test_tags=activate,activate:hook,activate:hook:bash
@test "bash: activate runs profile scripts" {
  # calls init
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="bash" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/hook.exp" "$PROJECT_DIR"
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  assert_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.zsh"

  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  assert_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.zsh"

  FLOX_NO_PROFILES=1 FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.zsh"

  # Turbo mode exec()s the provided command without involving the
  # userShell, so cannot invoke shell primitives like ":".
  FLOX_TURBO=1 FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run -127 $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_failure
  FLOX_TURBO=1 FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- true
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.zsh"

}

# bats test_tags=activate,activate:hook,activate:hook:zsh
@test "zsh: activate runs profile scripts" {
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  # FLOX_SHELL="zsh" USER="$REAL_USER" run -0 bash -c "echo exit | $FLOX_CLI activate --dir $PROJECT_DIR";
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/hook.exp" "$PROJECT_DIR"
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  assert_output --partial "sourcing profile.zsh"

  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  assert_output --partial "sourcing profile.zsh"

  FLOX_NO_PROFILES=1 FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.zsh"

  # Turbo mode exec()s the provided command without involving the
  # userShell, so cannot invoke shell primitives like ":".
  FLOX_TURBO=1 FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -127 $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_failure
  FLOX_TURBO=1 FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- true
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.zsh"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:rc:bash
@test "bash: activate respects ~/.bashrc" {
  echo "alias test_alias='echo testing'" > "$HOME/.bashrc"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
  assert_output --partial "test_alias is aliased to \`echo testing'"
}

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

  FLOX_SHELL="bash" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/envVar.exp" "$PROJECT_DIR"
  assert_output --partial "baz"

  FLOX_SHELL="bash" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- echo '$foo'
  assert_success
  assert_output --partial "baz"
}

# bats test_tags=activate,activate:envVar:zsh
@test "zsh: activate sets env var" {
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/envVar.exp" "$PROJECT_DIR"
  assert_output --partial "baz"

  FLOX_SHELL="zsh" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- echo '$foo'
  assert_success
  assert_output --partial "baz"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:envVar-before-hook:zsh
@test "zsh and bash: activate sets env var before hook" {
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="zsh" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
  FLOX_SHELL="bash" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:path,activate:path:bash
@test "'flox activate' modifies path (bash)" {
  original_path="$PATH"
  FLOX_SHELL="bash" run "$FLOX_BIN" activate -- echo '$PATH'
  assert_success
  assert_not_equal "$original_path" "$output"

  # hello is not on the path
  run -1 type hello

  run "$FLOX_BIN" install hello
  assert_success

  FLOX_SHELL="bash" run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"
}

# bats test_tags=activate,activate:path,activate:path:zsh
@test "'flox activate' modifies path (zsh)" {
  original_path="$PATH"
  FLOX_SHELL="zsh" run "$FLOX_BIN" activate -- echo '$PATH'
  assert_success
  assert_not_equal "$original_path" "$output"

  # hello is not on the path
  run -1 type hello

  run "$FLOX_BIN" install hello
  assert_success

  FLOX_SHELL="zsh" run "$FLOX_BIN" activate -- hello
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
  assert_line --partial "Disable command hashing"
}

# bats test_tags=activate,activate:inplace-prints
@test "'flox activate' prints script to modify current shell (zsh)" {
  # Flox detects that the output is not a tty and prints the script to stdout
  FLOX_SHELL="zsh" run "$FLOX_BIN" activate
  assert_success
  # check that env vars are set for compatibility with nix built software
  assert_line --partial "export NIX_SSL_CERT_FILE="
  assert_line --partial "Disable command hashing"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:inplace-modifies,activate:inplace-modifies:bash
@test "'flox activate' modifies the current shell (bash)" {
  # set profile scripts
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set a hook
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set vars
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  "$FLOX_BIN" install hello

  run bash -c 'eval "$($FLOX_BIN activate)"; type hello; echo $foo'
  assert_success
  assert_line "sourcing hook.on-activate"
  assert_line "sourcing profile.common"
  assert_line "sourcing profile.bash"
  refute_line "sourcing profile.zsh"
  assert_line --partial "hello is $(realpath $PROJECT_DIR)/.flox/run/"
  assert_line "baz"
}

# bats test_tags=activate,activate:inplace-modifies,activate:inplace-modifies:zsh
@test "'flox activate' modifies the current shell (zsh)" {
  # set profile scripts
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set a hook
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set vars
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  "$FLOX_BIN" install hello

  run zsh -c 'eval "$("$FLOX_BIN" activate)"; type hello; echo $foo'
  assert_success
  assert_line "sourcing hook.on-activate"
  assert_line "sourcing profile.common"
  refute_line "sourcing profile.bash"
  assert_line "sourcing profile.zsh"
  assert_line --partial "hello is $(realpath $PROJECT_DIR)/.flox/run/"
  assert_line "baz"
}

# ---------------------------------------------------------------------------- #

# N.B. removed "'flox activate' only patches PATH when already activated" test,
# because we in fact need to patch PATH with every activation to defend against
# userShell "dotfiles" that may have modified PATH unconditionally (e.g. on any
# host running nix-darwin(!)).
#
# Replacing it with a test that checks that a) activation puts the FLOX_ENV/bin
# first, and b) that it doesn't put it there more than once.

# bats test_tags=activate,activate:inplace-reactivate,activate:inplace-reactivate:bash
@test "bash: 'flox activate' patches PATH correctly when already activated" {
  FLOX_SHELL="bash" run -- \
    "$FLOX_BIN" activate -- \
      bash -c 'eval "$($FLOX_BIN activate)"; bash "$TESTS_DIR"/activate/verify_PATH.bash'
  assert_success
}

# bats test_tags=activate,activate:inplace-reactivate,activate:inplace-reactivate:zsh
@test "zsh: 'flox activate' patches PATH correctly when already activated" {
  FLOX_SHELL="zsh" run -- \
    "$FLOX_BIN" activate -- \
      zsh -c 'eval "$($FLOX_BIN activate)"; bash "$TESTS_DIR"/activate/verify_PATH.bash'
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:python-detects-installed-python
@test "'flox activate' sets python vars if python is installed" {
  # unset python vars if any
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

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:bash
@test "'hook.on-activate' modifies environment variables (bash)" {
  "$FLOX_BIN" delete -f
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"
  # Run a command that causes the activation scripts to run without entering
  # an interactive shell
  # What this is testing:
  # - The [vars] section sets foo=bar
  # - The on-activate script exports foo=baz
  # - We echo $foo from within userShell and see "baz" as expected
  SHELL="$(which bash)" run --separate-stderr "$FLOX_BIN" activate -- echo '$foo'
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "baz"
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:zsh
@test "'hook.on-activate' modifies environment variables (zsh)" {
  "$FLOX_BIN" delete -f
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"
  # Run a command that causes the activation scripts to run without entering
  # an interactive shell
  # What this is testing:
  # - The [vars] section sets foo=bar
  # - The on-activate script exports foo=baz
  # - We echo $foo from within userShell and see "baz" as expected
  SHELL="$(which zsh)" run --separate-stderr "$FLOX_BIN" activate -- echo '$foo'
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "baz"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:bash
@test "bash: 'hook.on-activate' is sourced before 'profile.common'" {
  "$FLOX_BIN" delete -f
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  run bash -c 'eval "$("$FLOX_BIN" activate)"'
  # 'hook.on-activate' sets a var containing "hookie",
  # 'profile.common' creates a directory named after the contents of that
  # variable, suffixed by '-common'
  [ -d "hookie-common" ]
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:zsh
@test "zsh: 'hook.on-activate' is sourced before 'profile.common'" {
  "$FLOX_BIN" delete -f
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  run zsh -c 'eval "$("$FLOX_BIN" activate)"'
  # 'hook.on-activate' sets a var containing "hookie",
  # 'profile.common' creates a directory named after the contents of that
  # variable, suffixed by '-common'
  [ -d "hookie-common" ]
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:bash
@test "bash: 'profile.common' is sourced before 'profile.bash'" {
  "$FLOX_BIN" delete -f
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  # N.B. we need the eval here because `bash -c` will otherwise
  # exec() flox and defeat the parent process detection.
  run bash -c 'eval "$("$FLOX_BIN" activate)"'
  # 'profile.common' sets a var containing "common",
  # 'profile.bash' creates a directory named after the contents of that
  # variable, suffixed by '-bash'
  [ -d "common-bash" ]
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:zsh
@test "zsh: 'profile.common' is sourced before 'profile.zsh'" {
  "$FLOX_BIN" delete -f
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  # N.B. we need the eval here because `zsh -c` will otherwise
  # exec() flox and defeat the parent process detection.
  run zsh -c 'eval "$("$FLOX_BIN" activate)"'
  # 'profile.common' sets a var containing "common",
  # 'profile.zsh' creates a directory named after the contents of that variable,
  # suffixed by '-zsh'
  [ -d "common-zsh" ]
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:paths_spaces,activate:paths_spaces:bash
@test "bash: tolerates paths containing spaces" {
  "$FLOX_BIN" delete -f
  bad_dir="contains space/project"
  mkdir -p "$PWD/$bad_dir"
  cd "$PWD/$bad_dir"
  "$FLOX_BIN" init
  run bash -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  refute_output --partial "no such file or directory"
}

# bats test_tags=activate,activate:paths_spaces,activate:paths_spaces:zsh
@test "zsh: tolerates paths containing spaces" {
  "$FLOX_BIN" delete -f
  bad_dir="contains space/project"
  mkdir -p "$PWD/$bad_dir"
  cd "$PWD/$bad_dir"
  "$FLOX_BIN" init
  run zsh -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  refute_output --partial "no such file or directory"
}

# ---------------------------------------------------------------------------- #
