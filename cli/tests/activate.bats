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

# Create a set of dotfiles to simulate the sorts of things users can do that
# disrupt flox's attempts to configure the environment. Please append to this
# growing list of nightmare scenarios as you encounter them in the wild.
user_dotfiles_setup() {
  # N.B. $HOME is set to a test-isolated directory by `common_file_setup`,
  # `home_setup`, and `flox_vars_setup` so none of the files below should exist
  # and we abort if we find otherwise.
  set -o noclobber

  BADPATH="/usr/local/bin:/usr/bin:/bin:/nix/var/nix/profiles/default/bin:/run/current-system/sw/bin"

  # Posix-compliant shells
  for i in "profile" "bashrc" \
    "zshrc" "zshenv" "zlogin" "zlogout" "zprofile"; do
    cat >"$HOME/.$i" <<EOF
echo "Sourcing .$i" >&2
echo "Setting PATH from .$i" >&2
export PATH="$BADPATH"
if [ -f "$HOME/.$i.extra" ]; then
  source "$HOME/.$i.extra";
fi
EOF
  done

  # Fish
  mkdir -p "$HOME/.config/fish"
  cat >"$HOME/.config/fish/config.fish" <<EOF
echo "Sourcing config.fish" >&2
echo "Setting PATH from config.fish" >&2
set -gx PATH "$BADPATH"
if test -e "$HOME/.config/fish/config.fish.extra"
  source "$HOME/.config/fish/config.fish.extra"
end
EOF

  # Csh-based shells
  for i in "cshrc" "tcshrc" "login" "logout"; do
    cat >"$HOME/.$i" <<EOF
sh -c "echo 'Sourcing .$i' >&2"
sh -c "echo 'Setting PATH from .$i' >&2"
setenv PATH "$BADPATH"
if ( -e "$HOME/.$i.extra" ) then
  source "$HOME/.$i.extra"
endif
EOF
  done

  set +o noclobber
}

setup_file() {
  common_file_setup
}

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup_common() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return

}

# setup with catalog
project_setup() {
  project_setup_common
  "$FLOX_BIN" init -d "$PROJECT_DIR"
}

# project setup with pkgdb
project_setup_pkgdb() {
  project_setup_common
  mkdir -p "$PROJECT_DIR/.flox/env"
  cp --no-preserve=mode "$MANUALLY_GENERATED"/hello_v0/* "$PROJECT_DIR/.flox/env"
  echo '{
    "name": "'$PROJECT_NAME'",
    "version": 1
  }' >>"$PROJECT_DIR/.flox/env.json"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  home_setup test # Isolate $HOME for each test.
  user_dotfiles_setup
  setup_isolated_flox # concurrent pkgdb database creation
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}
teardown() {
  # Cleaning up the `BATS_TEST_TMPDIR` occasionally fails,
  # because of an 'env-registry.json' that gets concurrently written
  # by the watchdog as the activation terminates.
  wait_for_watchdogs
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# Some constants

export VARS=$(
  cat <<EOF
[vars]
foo = "baz"
EOF
)

export HELLO_PROFILE_SCRIPT=$(
  cat <<-EOF
[profile]
common = """
  echo "sourcing profile.common";
"""
bash = """
  echo "sourcing profile.bash";
"""
fish = """
  echo "sourcing profile.fish";
"""
tcsh = """
  echo "sourcing profile.tcsh";
"""
zsh = """
  echo "sourcing profile.zsh";
"""
EOF
)

export VARS_HOOK_SCRIPT=$(
  cat <<EOF
[hook]
on-activate = """
  echo "sourcing hook.on-activate";
"""
EOF
)

export VARS_HOOK_SCRIPT_ECHO_FOO=$(
  cat <<EOF
[hook]
on-activate = """
  echo "sourcing hook.on-activate";
  echo \$foo;
"""
EOF
)

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:flox_shell,activate:flox_shell:bash
@test "activate identifies FLOX_SHELL from running shell (bash)" {
  project_setup
  run --separate-stderr bash -c "$FLOX_BIN activate | grep -- 'source .*/activate.d/'"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_line --partial "/activate.d/bash"
}

# bats test_tags=activate,activate:flox_shell,activate:flox_shell:fish
@test "activate identifies FLOX_SHELL from running shell (fish)" {
  project_setup
  run --separate-stderr fish -c "$FLOX_BIN activate | grep -- 'source .*/activate.d/'"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_line --partial "/activate.d/fish"
}

# bats test_tags=activate,activate:flox_shell,activate:flox_shell:tcsh
@test "activate identifies FLOX_SHELL from running shell (tcsh)" {
  project_setup
  run --separate-stderr tcsh -c "$FLOX_BIN activate | grep -- 'source .*/activate.d/'"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_line --partial "/activate.d/tcsh"
}

# bats test_tags=activate,activate:flox_shell,activate:flox_shell:zsh
@test "activate identifies FLOX_SHELL from running shell (zsh)" {
  project_setup
  run --separate-stderr zsh -c "$FLOX_BIN activate | grep -- 'source .*/activate.d/'"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_line --partial "/activate.d/zsh"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:path,activate:path:bash
@test "bash: interactive activate puts package in path" {
  project_setup_pkgdb
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:bash
@test "catalog: bash: interactive activate puts package in path" {
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:fish
@test "fish: interactive activate puts package in path" {
  project_setup_pkgdb
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:fish
@test "catalog: fish: interactive activate puts package in path" {
  project_setup_pkgdb
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:tcsh
@test "tcsh: interactive activate puts package in path" {
  project_setup_pkgdb
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:tcsh
@test "catalog: tcsh: interactive activate puts package in path" {
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:zsh
@test "zsh: interactive activate puts package in path" {
  project_setup_pkgdb
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:zsh
@test "catalog: zsh: interactive activate puts package in path" {
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# ---------------------------------------------------------------------------- #

# The following battery of tests ensure that the activation script invokes
# the expected hook and profile scripts for the bash and zsh shells, and
# in each of the following four scenarios:
#
# 1. in the interactive case, simulated using using `activate.exp`
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
  project_setup
  # calls init
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="bash" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  assert_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  assert_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  FLOX_NOPROFILE=1 FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  FLOX_TURBO=1 FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  FLOX_TURBO=1 FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- true
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly in various forms.
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -c :
  assert_success
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --command :
  assert_success
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -c true
  assert_success
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --command true
  assert_success
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate :
  assert_success
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -- :
  assert_success
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate true
  assert_success
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -- true
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  assert_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly with --noprofile.
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --noprofile :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly with --turbo.
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --turbo :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:fish
@test "fish: activate runs profile scripts" {
  project_setup
  # calls init
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="fish" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  assert_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  assert_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  FLOX_NOPROFILE=1 FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  FLOX_TURBO=1 FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  FLOX_TURBO=1 FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- true
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly in various forms.
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -c :
  assert_success
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --command :
  assert_success
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -c true
  assert_success
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --command true
  assert_success
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate :
  assert_success
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -- :
  assert_success
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate true
  assert_success
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -- true
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  assert_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly with --noprofile.
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --noprofile :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly with --turbo.
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --turbo :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:tcsh
@test "tcsh: activate runs profile scripts" {
  project_setup
  # calls init
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="tcsh" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  assert_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  assert_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  FLOX_NOPROFILE=1 FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  FLOX_TURBO=1 FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  FLOX_TURBO=1 FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- true
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly in various forms.
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -c :
  assert_success
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --command :
  assert_success
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -c true
  assert_success
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --command true
  assert_success
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate :
  assert_success
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -- :
  assert_success
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate true
  assert_success
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -- true
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  assert_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly with --noprofile.
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --noprofile :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly with --turbo.
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --turbo :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:zsh
@test "zsh: activate runs profile scripts" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  # FLOX_SHELL="zsh" USER="$REAL_USER" run -0 bash -c "echo exit | $FLOX_CLI activate --dir $PROJECT_DIR";
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  assert_output --partial "sourcing profile.zsh"

  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  assert_output --partial "sourcing profile.zsh"

  FLOX_NOPROFILE=1 FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  FLOX_TURBO=1 FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  FLOX_TURBO=1 FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- true
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly in various forms.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -c :
  assert_success
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --command :
  assert_success
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -c true
  assert_success
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --command true
  assert_success
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate :
  assert_success
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -- :
  assert_success
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate true
  assert_success
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate -- true
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  assert_output --partial "sourcing profile.zsh"

  # Test running the activate script directly with --noprofile.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --noprofile :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"

  # Test running the activate script directly with --turbo.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/activate --turbo :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:hook,activate:hook:bash
@test "bash: activate runs hook only once in nested activation" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [hook]
    on-activate = """
      echo "sourcing hook.on-activate"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # Don't use run or assert_output because we can't use them for
  # shells other than bash.
  cat <<'EOF' | bash
    eval "$("$FLOX_BIN" activate 2>"$PROJECT_DIR/stderr_1")"
    [[ "$(cat "$PROJECT_DIR/stderr_1")" == *"sourcing hook.on-activate"* ]]
    eval "$("$FLOX_BIN" activate 2>"$PROJECT_DIR/stderr_2")"
    [[ "$(cat "$PROJECT_DIR/stderr_2")" != *"sourcing hook.on-activate"* ]]
EOF
}

# bats test_tags=activate,activate:hook,activate:hook:fish
@test "fish: activate runs hook only once in nested activation" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [hook]
    on-activate = """
      echo "sourcing hook.on-activate"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # Don't use run or assert_output because we can't use them for
  # shells other than bash.
  cat <<'EOF' | fish
    eval "$("$FLOX_BIN" activate 2>"$PROJECT_DIR/stderr_1")"
    grep -q "sourcing hook.on-activate" "$PROJECT_DIR/stderr_1"
    eval "$("$FLOX_BIN" activate 2>"$PROJECT_DIR/stderr_2")"
    if grep -q "sourcing hook.on-activate" "$PROJECT_DIR/stderr_2"
      exit 1
    end
EOF
}

# bats test_tags=activate,activate:hook,activate:hook:tcsh
@test "tcsh: activate runs hook only once in nested activation" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [hook]
    on-activate = """
      echo "sourcing hook.on-activate"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # Don't use run or assert_output because we can't use them for
  # shells other than bash.
  cat <<'EOF' | tcsh
    eval "`$FLOX_BIN activate`" >& "$PROJECT_DIR/stderr_1"
    grep -q "sourcing hook.on-activate" "$PROJECT_DIR/stderr_1"
    "$FLOX_BIN" activate | grep -q "sourcing hook.on-activate"
    if ($? == 0) then
      exit 1
    endif
EOF
}

# bats test_tags=activate,activate:hook,activate:hook:zsh
@test "zsh: activate runs hook only once in nested activations" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [hook]
    on-activate = """
      echo "sourcing hook.on-activate"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # Don't use run or assert_output because we can't use them for
  # shells other than bash.
  cat <<'EOF' | zsh
    eval "$("$FLOX_BIN" activate 2>"$PROJECT_DIR/stderr_1")"
    [[ "$(cat "$PROJECT_DIR/stderr_1")" == *"sourcing hook.on-activate"* ]]
    eval "$("$FLOX_BIN" activate 2>"$PROJECT_DIR/stderr_2")"
    [[ "$(cat "$PROJECT_DIR/stderr_2")" != *"sourcing hook.on-activate"* ]]
EOF
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:hook,activate:hook:bash
@test "bash: activate runs profile twice in nested activation" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [profile]
    bash = """
      echo "sourcing profile.bash"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # Don't use run or assert_output because we can't use them for
  # shells other than bash.
  {
    output="$(FLOX_SHELL="bash" eval "$("$FLOX_BIN" activate)")"
    [[ "$output" == *"sourcing profile.bash"* ]]
    output="$(FLOX_SHELL="bash" eval "$("$FLOX_BIN" activate)")"
    [[ "$output" == *"sourcing profile.bash"* ]]
  }
}

# bats test_tags=activate,activate:hook,activate:hook:fish
@test "fish: activate runs profile twice in nested activation" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [profile]
    fish = """
      echo "sourcing profile.fish"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # TODO: this gives unhelpful failures
  cat <<'EOF' | fish
    set output "$(eval "$("$FLOX_BIN" activate)")"
    echo "$output" | string match "sourcing profile.fish"
    set output "$(eval "$("$FLOX_BIN" activate)")"
    echo "$output" | string match "sourcing profile.fish"
EOF
}

# bats test_tags=activate,activate:hook,activate:hook:tcsh
@test "tcsh: activate runs profile twice in nested activation" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [profile]
    tcsh = """
      echo "sourcing profile.tcsh"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # Don't use run or assert_output because we can't use them for
  # shells other than bash.
  cat <<'EOF' | tcsh
    eval "`$FLOX_BIN activate`" |& grep -q "sourcing profile.tcsh"
    eval "`$FLOX_BIN activate`" |& grep -q "sourcing profile.tcsh"
EOF
}

# bats test_tags=activate,activate:hook,activate:hook:zsh
@test "zsh: activate runs profile twice in nested activation" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [profile]
    zsh = """
      echo "sourcing profile.zsh"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # TODO: this gives unhelpful failures
  cat <<'EOF' | zsh
    output="$(FLOX_SHELL="zsh" eval "$("$FLOX_BIN" activate)")"
    [[ "$output" == *"sourcing profile.zsh"* ]]
    output="$(FLOX_SHELL="zsh" eval "$("$FLOX_BIN" activate)")"
    [[ "$output" == *"sourcing profile.zsh"* ]]
EOF
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:once
@test "activate runs hook and profile scripts only once" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  echo '# Testing non-interactive bash' >&2
  FLOX_SHELL="bash" NO_COLOR=1 run "$FLOX_BIN" activate -- :
  assert_success
  refute_output --partial "ERROR"
  assert_output --partial "sourcing hook.on-activate for first time"
  assert_output --partial "sourcing profile.bash for first time"
  refute_output --partial "sourcing profile.zsh for first time"

  echo '# Testing interactive bash' >&2
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
  assert_success
  refute_output --partial "ERROR"
  assert_output --partial "sourcing hook.on-activate for first time"
  assert_output --partial "sourcing profile.bash for first time"
  refute_output --partial "sourcing profile.zsh for first time"

  echo '# Testing non-interactive zsh' >&2
  FLOX_SHELL="zsh" NO_COLOR=1 run "$FLOX_BIN" activate -- :
  assert_success
  refute_output --partial "ERROR"
  assert_output --partial "sourcing hook.on-activate for first time"
  refute_output --partial "sourcing profile.bash for first time"
  assert_output --partial "sourcing profile.zsh for first time"

  echo '# Testing interactive zsh' >&2
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
  assert_success
  refute_output --partial "ERROR"
  assert_output --partial "sourcing hook.on-activate for first time"
  refute_output --partial "sourcing profile.bash for first time"
  assert_output --partial "sourcing profile.zsh for first time"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:rc:bash
@test "bash: activate respects ~/.bashrc" {
  project_setup
  echo "alias test_alias='echo testing'" >"$HOME/.bashrc.extra"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="bash" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
  assert_output --partial "test_alias is aliased to \`echo testing'"
}

# bats test_tags=activate,activate:fish,activate:rc:fish
@test "fish: activate respects ~/.config/fish/config.fish" {
  project_setup
  echo "alias test_alias='echo testing'" >"$HOME/.config/fish/config.fish.extra"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="fish" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
  # fish's liberal use of color codes forces us to use regex matching here,
  # and I've given up trying to match the single quotes. Here's the output
  # we're trying to match:
  #
  # function test_alias --wraps='echo testing' --description 'alias test_alias=echo testing'
  #
  # TODO: come up with a way to invoke fish with the "No colors" theme.
  assert_output --regexp \
    'function.*test_alias.*--wraps=.*echo testing.*--description.*alias test_alias=echo testing'
}

# bats test_tags=activate,activate:rc:tcsh
@test "tcsh: activate respects ~/.tcshrc" {
  project_setup
  echo 'alias test_alias "echo testing"' >"$HOME/.tcshrc.extra"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="tcsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc-tcsh.exp" "$PROJECT_DIR"
  assert_line --partial "echo testing"
}

# bats test_tags=activate,activate:rc:zsh
@test "zsh: activate respects ~/.zshrc" {
  project_setup
  echo "alias test_alias='echo testing'" >"$HOME/.zshrc.extra"
  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
  assert_output --partial "test_alias is an alias for echo testing"
}

# bats test_tags=activate,activate:rc:zsh
@test "zsh: interactive activate respects history settings from dotfile" {
  project_setup

  # This should always work, even when Darwin sets a default in `/etc/zshrc`.
  echo 'HISTFILE=${PROJECT_DIR}/.alt_history' >"$HOME/.zshrc.extra"
  echo 'SHELL_SESSION_DIR=${PROJECT_DIR}/.alt_sessions' >>"$HOME/.zshrc.extra"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 \
    run expect "$TESTS_DIR/activate/histfile.exp" "$PROJECT_DIR"
  assert_success
  assert_line --partial "HISTFILE=$PROJECT_DIR/.alt_history"
  assert_line --partial "SHELL_SESSION_DIR=$PROJECT_DIR/.alt_sessions"

  # Additionally it should never be an immutable storepath.
  refute_line --partial "HISTFILE=/nix/store/"
  refute_line --partial "SHELL_SESSION_DIR=/nix/store/"
}

# bats test_tags=activate,activate:rc:zsh
@test "zsh: interactive activate respects history settings from dotfile based on original ZDOTDIR" {
  project_setup

  # Mimic the default `/etc/zshrc` on Darwin prior to Nix being installed. We
  # have to do this in `~/.zshrc` because we can't mock `/etc/zshrc`. However we
  # apply the same `ZDOTDIR` logic to both, ensuring that it's not pointing at
  # our immutable storepath.
  echo 'HISTFILE=${ZDOTDIR:-$HOME}/.alt_history' >"$HOME/.zshrc.extra"
  echo 'SHELL_SESSION_DIR=${ZDOTDIR:-$HOME}/.alt_sessions' >>"$HOME/.zshrc.extra"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 \
    run expect "$TESTS_DIR/activate/histfile.exp" "$PROJECT_DIR"
  assert_success
  assert_line --partial "HISTFILE=$HOME/.alt_history"
  assert_line --partial "SHELL_SESSION_DIR=$HOME/.alt_sessions"

  # Additionally it should never be an immutable storepath.
  refute_line --partial "HISTFILE=/nix/store/"
  refute_line --partial "SHELL_SESSION_DIR=/nix/store/"
}

# bats test_tags=activate,activate:rc:zsh
@test "zsh: interactive activate respects history settings from environment variable where available" {
  project_setup

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="zsh" USER="$REAL_USER" NO_COLOR=1 \
    HISTFILE="$PROJECT_DIR/.alt_history" \
    SHELL_SESSION_DIR="$PROJECT_DIR/.alt_sessions" \
    run expect "$TESTS_DIR/activate/histfile.exp" "$PROJECT_DIR"
  assert_success

  # If the host configuration honours the environment variables then we do too.
  #
  # The majority of Linux distros don't ship with a default `/etc/zshrc` and will
  # return our custom value. So we expect a custom value when using Flox.
  #
  # Darwin, with or without Nix, ships with a default `/etc/zshrc` that returns
  # something other than our custom value. So we expect that default when using
  # Flox.
  CUSTOM_VALUE="/dev/null"
  HISTFILE_DEFAULT="$(HISTFILE=$CUSTOM_VALUE zsh -ic 'echo $HISTFILE')"
  SHELL_SESSION_DIR_DEFAULT="$(SHELL_SESSION_DIR=$CUSTOM_VALUE zsh -ic 'echo $SHELL_SESSION_DIR')"

  if [[ "$HISTFILE_DEFAULT" == "$CUSTOM_VALUE" ]]; then
    assert_line --partial "HISTFILE=${PROJECT_DIR}/.alt_history"
  else
    assert_line --partial "HISTFILE=${HISTFILE_DEFAULT}"
  fi

  if [[ "$SHELL_SESSION_DIR_DEFAULT=" == "$CUSTOM_VALUE" ]]; then
    assert_line --partial "SHELL_SESSION_DIR=${PROJECT_DIR}/.alt_sessions"
  else
    assert_line --partial "SHELL_SESSION_DIR=${SHELL_SESSION_DEFAULT}"
  fi

  # Additionally it should never be an immutable storepath.
  refute_line --partial "HISTFILE=/nix/store/"
  refute_line --partial "SHELL_SESSION_DIR=/nix/store/"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:envVar:bash
@test "bash: activate sets env var" {
  project_setup
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="bash" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/envVar.exp" "$PROJECT_DIR"
  assert_output --partial "baz"

  FLOX_SHELL="bash" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- echo '$foo'
  assert_success
  assert_output --partial "baz"
}

# bats test_tags=activate,activate:envVar:fish
@test "fish: activate sets env var" {
  project_setup
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="fish" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/envVar.exp" "$PROJECT_DIR"
  assert_output --partial "baz"

  FLOX_SHELL="fish" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- echo '$foo'
  assert_success
  assert_output --partial "baz"
}

# bats test_tags=activate,activate:envVar:tcsh
@test "tcsh: activate sets env var" {
  project_setup
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="tcsh" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/envVar.exp" "$PROJECT_DIR"
  assert_output --partial "baz"

  FLOX_SHELL="tcsh" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- echo '$foo'
  assert_success
  assert_output --partial "baz"
}

# bats test_tags=activate,activate:envVar:zsh
@test "zsh: activate sets env var" {
  project_setup
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

# bats test_tags=activate,activate:envVar-before-hook
@test "{bash,fish,tcsh,zsh}: activate sets env var before hook" {
  project_setup
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT_ECHO_FOO//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # TODO: flox will set HOME if it doesn't match the home of the user with
  # current euid. I'm not sure if we should change that, but for now just set
  # USER to REAL_USER.
  FLOX_SHELL="bash" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
  FLOX_SHELL="fish" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
  FLOX_SHELL="tcsh" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
  FLOX_SHELL="zsh" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:path,activate:path:bash
@test "'flox activate' modifies path (bash)" {
  project_setup_pkgdb

  # hello is not on the path
  run -1 type hello

  # project_setup_pkgdb sets up an environment with hello installed
  FLOX_SHELL="bash" run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"
}

# bats test_tags=activate,activate:path,activate:path:bash
@test "catalog: 'flox activate' modifies path (bash)" {
  project_setup
  original_path="$PATH"
  FLOX_SHELL="bash" run "$FLOX_BIN" activate -- echo '$PATH'
  assert_success
  assert_not_equal "$original_path" "$output"

  # hello is not on the path
  run -1 type hello

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install hello
  assert_success

  FLOX_SHELL="bash" run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"
}

# bats test_tags=activate,activate:path,activate:path:fish
@test "'flox activate' modifies path (fish)" {
  project_setup_pkgdb

  # hello is not on the path
  run -1 type hello

  # project_setup_pkgdb sets up an environment with hello installed
  FLOX_SHELL="fish" run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"
}

# bats test_tags=activate,activate:path,activate:path:fish
@test "catalog: 'flox activate' modifies path (fish)" {
  project_setup
  original_path="$PATH"
  FLOX_SHELL="fish" run "$FLOX_BIN" activate -- echo '$PATH'
  assert_success
  assert_not_equal "$original_path" "$output"

  # hello is not on the path
  run -1 type hello

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install hello
  assert_success

  FLOX_SHELL="fish" run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"
}

# bats test_tags=activate,activate:path,activate:path:tcsh
@test "'flox activate' modifies path (tcsh)" {
  project_setup_pkgdb

  # hello is not on the path
  run -1 type hello

  # project_setup_pkgdb sets up an environment with hello installed
  FLOX_SHELL="tcsh" run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"
}

# bats test_tags=activate,activate:path,activate:path:tcsh
@test "catalog: 'flox activate' modifies path (tcsh)" {
  project_setup
  original_path="$PATH"
  FLOX_SHELL="tcsh" run "$FLOX_BIN" activate -- echo '$PATH'
  assert_success
  assert_not_equal "$original_path" "$output"

  # hello is not on the path
  run -1 type hello

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install hello
  assert_success

  FLOX_SHELL="tcsh" run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"
}

# bats test_tags=activate,activate:path,activate:path:zsh
@test "'flox activate' modifies path (zsh)" {
  project_setup_pkgdb

  # hello is not on the path
  run -1 type hello

  # project_setup_pkgdb sets up an environment with hello installed
  FLOX_SHELL="zsh" run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"
}

# bats test_tags=activate,activate:path,activate:path:zsh
@test "catalog: 'flox activate' modifies path (zsh)" {
  project_setup
  original_path="$PATH"
  FLOX_SHELL="zsh" run "$FLOX_BIN" activate -- echo '$PATH'
  assert_success
  assert_not_equal "$original_path" "$output"

  # hello is not on the path
  run -1 type hello

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install hello
  assert_success

  FLOX_SHELL="zsh" run "$FLOX_BIN" activate -- hello
  assert_success
  assert_output --partial "Hello, world!"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:inplace-prints
@test "'flox activate' prints script to modify current shell (bash)" {
  project_setup
  # Flox detects that the output is not a tty and prints the script to stdout
  FLOX_SHELL="bash" run "$FLOX_BIN" activate
  assert_success
  # check that env vars are set for compatibility with nix built software
  assert_line --partial "export NIX_SSL_CERT_FILE="
  assert_line --partial "activate.d/bash"
}

# bats test_tags=activate,activate:inplace-prints
@test "'flox activate' prints script to modify current shell (fish)" {
  project_setup
  # Flox detects that the output is not a tty and prints the script to stdout
  FLOX_SHELL="fish" run "$FLOX_BIN" activate
  assert_success
  # check that env vars are set for compatibility with nix built software
  assert_line --partial "set -gx NIX_SSL_CERT_FILE "
  assert_line --partial "activate.d/fish"
}

# bats test_tags=activate,activate:inplace-prints
@test "'flox activate' prints script to modify current shell (tcsh)" {
  project_setup
  # Flox detects that the output is not a tty and prints the script to stdout
  FLOX_SHELL="tcsh" run "$FLOX_BIN" activate
  assert_success
  # check that env vars are set for compatibility with nix built software
  assert_line --partial "setenv NIX_SSL_CERT_FILE "
  assert_line --partial "activate.d/tcsh"
}

# bats test_tags=activate,activate:inplace-prints
@test "'flox activate' prints script to modify current shell (zsh)" {
  project_setup
  # Flox detects that the output is not a tty and prints the script to stdout
  FLOX_SHELL="zsh" run "$FLOX_BIN" activate
  assert_success
  # check that env vars are set for compatibility with nix built software
  assert_line --partial "export NIX_SSL_CERT_FILE="
  assert_line --partial "activate.d/zsh"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:inplace-modifies,activate:inplace-modifies:bash
@test "'flox activate' modifies the current shell (bash)" {
  project_setup_pkgdb

  cp -r "$MANUALLY_GENERATED"/hello_for_activate_v0/* .flox/env/

  run bash -c 'eval "$($FLOX_BIN activate)"; type hello; echo $foo'
  assert_success
  assert_line "sourcing hook.on-activate"
  assert_line "sourcing profile.common"
  assert_line "sourcing profile.bash"
  refute_line "sourcing profile.fish"
  refute_line "sourcing profile.tcsh"
  refute_line "sourcing profile.zsh"
  assert_line --partial "hello is $(realpath $PROJECT_DIR)/.flox/run/"
  assert_line "baz"
}

# bats test_tags=activate,activate:inplace-modifies,activate:inplace-modifies:bash
@test "catalog: 'flox activate' modifies the current shell (bash)" {
  project_setup
  # set profile scripts
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set a hook
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set vars
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
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

# bats test_tags=activate,activate:inplace-modifies,activate:inplace-modifies:fish
@test "'flox activate' modifies the current shell (fish)" {
  project_setup_pkgdb

  cp -r "$MANUALLY_GENERATED"/hello_for_activate_v0/* .flox/env/

  run fish -c 'eval "$($FLOX_BIN activate)"; type hello; echo $foo'
  assert_success
  assert_line "sourcing hook.on-activate"
  assert_line "sourcing profile.common"
  refute_line "sourcing profile.bash"
  assert_line "sourcing profile.fish"
  refute_line "sourcing profile.tcsh"
  refute_line "sourcing profile.zsh"
  assert_line --partial "hello is $(realpath $PROJECT_DIR)/.flox/run/"
  assert_line "baz"
}

# bats test_tags=activate,activate:inplace-modifies,activate:inplace-modifies:fish
@test "catalog: 'flox activate' modifies the current shell (fish)" {
  project_setup
  # set profile scripts
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set a hook
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set vars
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" install hello

  run fish -c 'eval "$($FLOX_BIN activate)"; type hello; echo $foo'
  assert_success
  assert_line "sourcing hook.on-activate"
  assert_line "sourcing profile.common"
  refute_line "sourcing profile.bash"
  assert_line "sourcing profile.fish"
  refute_line "sourcing profile.tcsh"
  refute_line "sourcing profile.zsh"
  assert_line --partial "hello is $(realpath $PROJECT_DIR)/.flox/run/"
  assert_line "baz"
}

# bats test_tags=activate,activate:inplace-modifies,activate:inplace-modifies:tcsh
@test "'flox activate' modifies the current shell (tcsh)" {
  project_setup_pkgdb

  cp -r "$MANUALLY_GENERATED"/hello_for_activate_v0/* .flox/env/

  run tcsh -c 'eval "`$FLOX_BIN activate`"; echo hello is `which hello`; echo $foo'
  assert_success
  assert_line "sourcing hook.on-activate"
  assert_line "sourcing profile.common"
  refute_line "sourcing profile.bash"
  refute_line "sourcing profile.fish"
  assert_line "sourcing profile.tcsh"
  refute_line "sourcing profile.zsh"
  assert_line --partial "hello is $(realpath $PROJECT_DIR)/.flox/run/"
  assert_line "baz"
}

# bats test_tags=activate,activate:inplace-modifies,activate:inplace-modifies:tcsh
@test "catalog: 'flox activate' modifies the current shell (tcsh)" {
  project_setup
  # set profile scripts
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set a hook
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set vars
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" install hello

  run tcsh -c 'eval "`$FLOX_BIN activate`"; echo hello is `which hello`; echo $foo'
  assert_success
  assert_line "sourcing hook.on-activate"
  assert_line "sourcing profile.common"
  refute_line "sourcing profile.bash"
  refute_line "sourcing profile.fish"
  assert_line "sourcing profile.tcsh"
  refute_line "sourcing profile.zsh"
  assert_line --partial "hello is $(realpath $PROJECT_DIR)/.flox/run/"
  assert_line "baz"
}

# bats test_tags=activate,activate:inplace-modifies,activate:inplace-modifies:zsh
@test "'flox activate' modifies the current shell (zsh)" {
  project_setup_pkgdb

  cp -r "$MANUALLY_GENERATED"/hello_for_activate_v0/* .flox/env/

  run zsh -c 'eval "$("$FLOX_BIN" activate)"; type hello; echo $foo'
  assert_success
  assert_line "sourcing hook.on-activate"
  assert_line "sourcing profile.common"
  refute_line "sourcing profile.bash"
  assert_line "sourcing profile.zsh"
  assert_line --partial "hello is $(realpath $PROJECT_DIR)/.flox/run/"
  assert_line "baz"
}

# bats test_tags=activate,activate:inplace-modifies,activate:inplace-modifies:zsh
@test "catalog: 'flox activate' modifies the current shell (zsh)" {
  project_setup
  # set profile scripts
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set a hook
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  # set vars
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" install hello

  run zsh -c 'eval "$("$FLOX_BIN" activate)"; type hello; echo $foo'
  assert_success
  assert_line "sourcing hook.on-activate"
  assert_line "sourcing profile.common"
  refute_line "sourcing profile.bash"
  refute_line "sourcing profile.fish"
  refute_line "sourcing profile.tcsh"
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
  project_setup
  FLOX_SHELL="bash" run -- \
    "$FLOX_BIN" activate -- \
    bash -c 'eval "$($FLOX_BIN activate)"; bash "$TESTS_DIR"/activate/verify_PATH.bash'
  assert_success
}

# bats test_tags=activate,activate:inplace-reactivate,activate:inplace-reactivate:fish
@test "fish: 'flox activate' patches PATH correctly when already activated" {
  project_setup
  FLOX_SHELL="fish" run -- \
    "$FLOX_BIN" activate -- \
    fish -c 'eval "$($FLOX_BIN activate)"; bash "$TESTS_DIR"/activate/verify_PATH.bash'
  assert_success
}

# bats test_tags=activate,activate:inplace-reactivate,activate:inplace-reactivate:tcsh
@test "tcsh: 'flox activate' patches PATH correctly when already activated" {
  project_setup
  # TODO: figure out why bats doesn't tolerate backticks in the following
  #       example, reports unmatched quotes. Going with this in the meantime
  #       because it works ...
  FLOX_SHELL="tcsh" run -- \
    "$FLOX_BIN" activate -- \
    tcsh -c "$FLOX_BIN activate | source /dev/stdin; bash $TESTS_DIR/activate/verify_PATH.bash"
  assert_success
}

# bats test_tags=activate,activate:inplace-reactivate,activate:inplace-reactivate:zsh
@test "zsh: 'flox activate' patches PATH correctly when already activated" {
  project_setup
  FLOX_SHELL="zsh" run -- \
    "$FLOX_BIN" activate -- \
    zsh -c 'eval "$($FLOX_BIN activate)"; bash "$TESTS_DIR"/activate/verify_PATH.bash'
  assert_success
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:python-detects-installed-python
@test "'flox activate' sets python vars if python is installed" {
  project_setup_pkgdb

  # Mock flox install of python311Packages.pip
  cp "$MANUALLY_GENERATED"/python_v0/* "$PROJECT_DIR/.flox/env/"

  # unset python vars if any
  unset PYTHONPATH
  unset PIP_CONFIG_FILE

  run -- "$FLOX_BIN" activate -- echo PYTHONPATH is '$PYTHONPATH'
  assert_success
  assert_line "PYTHONPATH is $(realpath $PROJECT_DIR)/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/lib/python3.11/site-packages"

  run -- "$FLOX_BIN" activate -- echo PIP_CONFIG_FILE is '$PIP_CONFIG_FILE'
  assert_success
  assert_line "PIP_CONFIG_FILE is $(realpath $PROJECT_DIR)/.flox/pip.ini"
}

# bats test_tags=activate,activate:python-detects-installed-python
@test "catalog: 'flox activate' sets python vars if python is installed" {
  project_setup
  # unset python vars if any
  unset PYTHONPATH
  unset PIP_CONFIG_FILE

  # install python and pip
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/python311Packages.pip.json"
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
  project_setup
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
@test "catalog: 'flox *' uses local environment over 'default' environment" {
  project_setup # TODO: we need PROJECT_DIR, but not flox init
  "$FLOX_BIN" delete -f
  mkdir default
  pushd default >/dev/null || return
  "$FLOX_BIN" init
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.json"
  "$FLOX_BIN" install vim
  popd >/dev/null || return

  "$FLOX_BIN" init
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/emacs.json"
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
  project_setup
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
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"
  # Run a command that causes the activation scripts to run without entering
  # an interactive shell
  # What this is testing:
  # - The [vars] section sets foo=bar
  # - The on-activate script exports foo=baz
  # - We echo $foo from within userShell and see "baz" as expected
  FLOX_SHELL="bash" run --separate-stderr "$FLOX_BIN" activate -- echo '$foo'
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "baz"
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:fish
@test "'hook.on-activate' modifies environment variables (fish)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"
  # Run a command that causes the activation scripts to run without entering
  # an interactive shell
  # What this is testing:
  # - The [vars] section sets foo=bar
  # - The on-activate script exports foo=baz
  # - We echo $foo from within userShell and see "baz" as expected
  SHELL="$(which fish)" run --separate-stderr "$FLOX_BIN" activate -- echo '$foo'
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "baz"
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:tcsh
@test "'hook.on-activate' modifies environment variables (tcsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"
  # Run a command that causes the activation scripts to run without entering
  # an interactive shell
  # What this is testing:
  # - The [vars] section sets foo=bar
  # - The on-activate script exports foo=baz
  # - We echo $foo from within userShell and see "baz" as expected
  SHELL="$(which tcsh)" run --separate-stderr "$FLOX_BIN" activate -- echo '$foo'
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "baz"
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:zsh
@test "'hook.on-activate' modifies environment variables (zsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"
  # Run a command that causes the activation scripts to run without entering
  # an interactive shell
  # What this is testing:
  # - The [vars] section sets foo=bar
  # - The on-activate script exports foo=baz
  # - We echo $foo from within userShell and see "baz" as expected
  FLOX_SHELL="zsh" run --separate-stderr "$FLOX_BIN" activate -- echo '$foo'
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "baz"
}

# ---------------------------------------------------------------------------- #

@test "'hook.on-activate' modifies environment variables in nested activation (bash)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"

  {
    eval "$("$FLOX_BIN" activate)"
    [[ "$foo" == baz ]]
    unset foo
    eval "$("$FLOX_BIN" activate)"
    [[ "$foo" == baz ]]
  }
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:fish
@test "'hook.on-activate' modifies environment variables in nested activation (fish)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"

  cat <<'EOF' | fish
    eval "$("$FLOX_BIN" activate)"
    echo "$foo" | string match "baz"
    set -e foo
    eval "$("$FLOX_BIN" activate)"
    echo "$foo" | string match "baz"
EOF
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:tcsh
@test "'hook.on-activate' modifies environment variables in nested activation (tcsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"

  cat <<'EOF' | tcsh -v
    eval "`$FLOX_BIN activate`"
    if ( "$foo" != baz ) then
      exit 1
    endif
    unsetenv foo
    eval "`$FLOX_BIN activate`"
    if ( "$foo" != baz ) then
      exit 1
    endif
EOF
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:zsh
@test "'hook.on-activate' modifies environment variables in nested activation (zsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"

  # TODO: this gives unhelpful failures
  cat <<'EOF' | zsh
    eval "$("$FLOX_BIN" activate)"
    [[ "$foo" == baz ]]
    unset foo
    eval "$("$FLOX_BIN" activate)"
    [[ "$foo" == baz ]]
EOF
}

# ---------------------------------------------------------------------------- #

@test "'hook.on-activate' unsets environment variables in nested activation (bash)" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [hook]
    on-activate = """
      unset foo
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  {
    export foo=baz
    eval "$(FLOX_SHELL="bash" "$FLOX_BIN" activate)"
    [[ -z "${foo:-}" ]]
    export foo=baz
    eval "$(FLOX_SHELL="bash" "$FLOX_BIN" activate)"
    [[ -z "${foo:-}" ]]
  }
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:fish
@test "'hook.on-activate' unsets environment variables in nested activation (fish)" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [hook]
    on-activate = """
      unset foo
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # TODO: this gives unhelpful failures
  cat <<'EOF' | fish
    set -gx foo baz
    eval "$("$FLOX_BIN" activate)"
    if set -q foo
      exit 1
    end
    set -gx foo baz
    eval "$("$FLOX_BIN" activate)"
    if set -q foo
      exit 1
    end
EOF
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:tcsh
@test "'hook.on-activate' unsets environment variables in nested activation (tcsh)" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [hook]
    on-activate = """
      unset foo
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # TODO: this gives unhelpful failures
  cat <<'EOF' | tcsh
    setenv foo baz
    eval "`$FLOX_BIN activate`"
    if ( $?foo ) then
      exit 1
    endif
    setenv foo baz
    eval "`$FLOX_BIN activate`"
    if ( $?foo ) then
      exit 1
    endif
EOF
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:zsh
@test "'hook.on-activate' unsets environment variables in nested activation (zsh)" {
  project_setup

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [hook]
    on-activate = """
      unset foo
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # TODO: this gives unhelpful failures
  cat <<'EOF' | zsh
    export foo=baz
    eval "$("$FLOX_BIN" activate)"
    [[ -z "${foo:-}" ]]
    export foo=baz
    eval "$("$FLOX_BIN" activate)"
    [[ -z "${foo:-}" ]]
EOF
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:bash
@test "bash: 'hook.on-activate' is sourced before 'profile.common'" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  run bash -c 'eval "$("$FLOX_BIN" activate)"'
  # 'hook.on-activate' sets a var containing "hookie",
  # 'profile.common' creates a directory named after the contents of that
  # variable, suffixed by '-common'
  [ -d "hookie-common" ]
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:fish
@test "fish: 'hook.on-activate' is sourced before 'profile.common'" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  run fish -c 'eval "$("$FLOX_BIN" activate)"'
  # 'hook.on-activate' sets a var containing "hookie",
  # 'profile.common' creates a directory named after the contents of that
  # variable, suffixed by '-common'
  [ -d "hookie-common" ]
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:tcsh
@test "tcsh: 'hook.on-activate' is sourced before 'profile.common'" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  run tcsh -c 'eval "`$FLOX_BIN activate`"'
  # 'hook.on-activate' sets a var containing "hookie",
  # 'profile.common' creates a directory named after the contents of that
  # variable, suffixed by '-common'
  [ -d "hookie-common" ]
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:zsh
@test "zsh: 'hook.on-activate' is sourced before 'profile.common'" {
  project_setup
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
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  # N.B. we need the eval here because `bash -c` will otherwise
  # exec() flox and defeat the parent process detection.
  run bash -c 'eval "$("$FLOX_BIN" activate)"'
  # 'profile.common' sets a var containing "common",
  # 'profile.bash' creates a directory named after the contents of that
  # variable, suffixed by '-bash'
  [ -d "common-bash" ]
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:fish
@test "fish: 'profile.common' is sourced before 'profile.fish'" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  # N.B. we need the eval here because `fish -c` will otherwise
  # exec() flox and defeat the parent process detection.
  run fish -c 'eval "$("$FLOX_BIN" activate)"'
  # 'profile.common' sets a var containing "common",
  # 'profile.fish' creates a directory named after the contents of that
  # variable, suffixed by '-fish'
  [ -d "common-fish" ]
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:tcsh
@test "tcsh: 'profile.common' is sourced before 'profile.tcsh'" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/profile-order.toml"
  # N.B. we need the eval here because `tcsh -c` will otherwise
  # exec() flox and defeat the parent process detection.
  run tcsh -c 'eval "`$FLOX_BIN activate`"'
  # 'profile.common' sets a var containing "common",
  # 'profile.tcsh' creates a directory named after the contents of that
  # variable, suffixed by '-tcsh'
  [ -d "common-tcsh" ]
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:zsh
@test "zsh: 'profile.common' is sourced before 'profile.zsh'" {
  project_setup
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
  project_setup # TODO: we need PROJECT_DIR, but not flox init
  bad_dir="contains space/project"
  mkdir -p "$PWD/$bad_dir"
  cd "$PWD/$bad_dir"
  "$FLOX_BIN" init
  run bash -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  refute_output --partial "no such file or directory"
}

# bats test_tags=activate,activate:paths_spaces,activate:paths_spaces:fish
@test "fish: tolerates paths containing spaces" {
  project_setup # TODO: we need PROJECT_DIR, but not flox init
  bad_dir="contains space/project"
  mkdir -p "$PWD/$bad_dir"
  cd "$PWD/$bad_dir"
  "$FLOX_BIN" init
  run fish -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  refute_output --partial "no such file or directory"
}

# bats test_tags=activate,activate:paths_spaces,activate:paths_spaces:tcsh
@test "tcsh: tolerates paths containing spaces" {
  project_setup # TODO: we need PROJECT_DIR, but not flox init
  bad_dir="contains space/project"
  mkdir -p "$PWD/$bad_dir"
  cd "$PWD/$bad_dir"
  "$FLOX_BIN" init
  run tcsh -c 'eval "`$FLOX_BIN activate`"'
  assert_success
  refute_output --partial "no such file or directory"
}

# bats test_tags=activate,activate:paths_spaces,activate:paths_spaces:zsh
@test "zsh: tolerates paths containing spaces" {
  project_setup # TODO: we need PROJECT_DIR, but not flox init
  bad_dir="contains space/project"
  mkdir -p "$PWD/$bad_dir"
  cd "$PWD/$bad_dir"
  "$FLOX_BIN" init
  run zsh -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  refute_output --partial "no such file or directory"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:infinite_source,activate:infinite_source:bash
@test "bash: test for infinite source loop" {
  project_setup
  # The bash -ic invocation sources .bashrc, and then the activate sources it a
  # second time and disables further sourcing.
  cat >"$HOME/.bashrc.extra" <<EOF
if [ -z "\$ALREADY_SOURCED" ]; then
  export ALREADY_SOURCED=1
elif [ "\$ALREADY_SOURCED" == 1 ]; then
  export ALREADY_SOURCED=2
else
  exit 2
fi

eval "\$("$FLOX_BIN" activate -d "$PWD")"
EOF
  bash -ic true
}

# bats test_tags=activate,activate:infinite_source,activate:infinite_source:fish
@test "fish: test for infinite source loop" {
  project_setup
  cat >"$HOME/.config/fish/config.fish.extra" <<EOF
if set -q ALREADY_SOURCED
  exit 2
end
set -gx ALREADY_SOURCED 1

eval "\$("$FLOX_BIN" activate -d "$PWD")"
EOF
  fish -ic true
}

# bats test_tags=activate,activate:infinite_source,activate:infinite_source:tcsh
@test "tcsh: test for infinite source loop" {
  project_setup
  cat >"$HOME/.tcshrc.extra" <<EOF
if ( \$?ALREADY_SOURCED ) then
  exit 2
endif
setenv ALREADY_SOURCED 1

eval "\`$FLOX_BIN activate -d $PWD\`"
EOF
  tcsh -ic true
}

# bats test_tags=activate,activate:infinite_source,activate:infinite_source:zsh
@test "zsh: test for infinite source loop" {
  project_setup
  cat >"$HOME/.zshrc.extra" <<EOF
if [ -n "\$ALREADY_SOURCED" ]; then
  exit 2
else
  export ALREADY_SOURCED=1
fi

eval "\$("$FLOX_BIN" activate -d "$PWD")"
EOF
  zsh -ic true
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:custom_zdotdir,activate:custom_zdotdir:bash
@test "bash: preserve custom ZDOTDIR" {
  project_setup
  FLOX_SHELL=bash ZDOTDIR=/custom/zdotdir run "$FLOX_BIN" activate -- echo '$ZDOTDIR'
  assert_success
  assert_line "/custom/zdotdir"
}

# bats test_tags=activate,activate:custom_zdotdir,activate:custom_zdotdir:fish
@test "fish: preserve custom ZDOTDIR" {
  project_setup
  FLOX_SHELL=fish ZDOTDIR=/custom/zdotdir run "$FLOX_BIN" activate -- echo '$ZDOTDIR'
  assert_success
  assert_line "/custom/zdotdir"
}

# bats test_tags=activate,activate:custom_zdotdir,activate:custom_zdotdir:tcsh
@test "tcsh: preserve custom ZDOTDIR" {
  project_setup
  FLOX_SHELL=tcsh ZDOTDIR=/custom/zdotdir run "$FLOX_BIN" activate -- echo '$ZDOTDIR'
  assert_success
  assert_line "/custom/zdotdir"
}

# bats test_tags=activate,activate:custom_zdotdir,activate:custom_zdotdir:zsh
@test "zsh: preserve custom ZDOTDIR" {
  project_setup
  FLOX_SHELL=zsh ZDOTDIR=/custom/zdotdir run "$FLOX_BIN" activate -- echo '$ZDOTDIR'
  assert_success
  assert_line "/custom/zdotdir"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:zdotdir,activate:zdotdir:zshenv
@test "zsh: in-place activation with non-interactive non-login shell" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  run zsh -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  assert_output - <<EOF
Sourcing .zshenv
Setting PATH from .zshenv
sourcing hook.on-activate for first time
sourcing profile.common for first time
sourcing profile.zsh for first time
EOF
  refute_output --partial "zprofile"
  refute_output --partial "zshrc"
  refute_output --partial "zlogin"
  refute_output --partial "zlogout"
}

# bats test_tags=activate,activate:zdotdir,activate:zdotdir:zshrc
@test "zsh: in-place activation with interactive non-login shell" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  run zsh --interactive -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  assert_output - <<EOF
Sourcing .zshenv
Setting PATH from .zshenv
Sourcing .zshrc
Setting PATH from .zshrc
sourcing hook.on-activate for first time
sourcing profile.common for first time
sourcing profile.zsh for first time
EOF
  refute_output --partial "zprofile"
  refute_output --partial "zlogin"
  refute_output --partial "zlogout"
}

# bats test_tags=activate,activate:zdotdir,activate:zdotdir:zlogin
@test "zsh: in-place activation with interactive login shell" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  run zsh --interactive --login -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  assert_output - <<EOF
Sourcing .zshenv
Setting PATH from .zshenv
Sourcing .zprofile
Setting PATH from .zprofile
Sourcing .zshrc
Setting PATH from .zshrc
Sourcing .zlogin
Setting PATH from .zlogin
sourcing hook.on-activate for first time
sourcing profile.common for first time
sourcing profile.zsh for first time
Sourcing .zlogout
Setting PATH from .zlogout
EOF
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:zdotdir,activate:zdotdir:zshenv
@test "zsh: in-place activation from .zshenv" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  echo 'eval "$("$FLOX_BIN" activate)"' >> "$HOME/.zshenv"

  run zsh --interactive --login -c 'true'
  assert_success
  assert_output - <<EOF
Sourcing .zshenv
Setting PATH from .zshenv
sourcing hook.on-activate for first time
sourcing profile.common for first time
sourcing profile.zsh for first time
Sourcing .zprofile
Setting PATH from .zprofile
Sourcing .zshrc
Setting PATH from .zshrc
Sourcing .zlogin
Setting PATH from .zlogin
Sourcing .zlogout
Setting PATH from .zlogout
EOF
}

# bats test_tags=activate,activate:zdotdir,activate:zdotdir:zshrc
@test "zsh: activation after in-place activation from .zshrc" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  # Undo `BADPATH` changes from `user_dotfiles_setup` so that we get binaries
  # from the `flox-cli-tests` devShell.
  run rm "$HOME"/.z*
  assert_success

  echo 'eval "$("$FLOX_BIN" activate)"' > "$HOME/.zshrc"

  "$FLOX_BIN" init -d nested
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [profile]
    common = """
      echo "nested profile.common"
    """
    zsh = """
      echo "nested profile.zsh"
    """
    [hook]
    on-activate = """
      echo "nested hook.on-activate"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -d nested -f -

  # Unset the flags from `only-once` profile scripts because we expect them to
  # be run once by the outer `zsh` command, then again by the inner `nested`
  # profile scripts, and then no further.
  # Also need to strip carriage-returns from the `expect` output in order for
  # BATS to do multi-line assertions on the output.
  FLOX_SHELL=zsh USER="$REAL_USER" NO_COLOR=1 run zsh --interactive --login -c \
    "unset _already_ran_profile_common _already_ran_profile_zsh && expect $TESTS_DIR/activate/activate.exp nested | tr -d '\r'"
  assert_success
  # Outer in-place activation.
  assert_output --partial - <<EOF
sourcing hook.on-activate for first time
sourcing profile.common for first time
sourcing profile.zsh for first time
EOF
  # Inner interactive activation.
  assert_output --partial - <<EOF
nested hook.on-activate
sourcing profile.common for first time
sourcing profile.zsh for first time
nested profile.common
nested profile.zsh
EOF
}

# bats test_tags=activate,activate:zdotdir,activate:zdotdir:zlogin
@test "zsh: in-place activation from .zlogin" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  echo 'eval "$("$FLOX_BIN" activate)"' >> "$HOME/.zlogin"

  run zsh --interactive --login -c 'true'
  assert_success
  assert_output - <<EOF
Sourcing .zshenv
Setting PATH from .zshenv
Sourcing .zprofile
Setting PATH from .zprofile
Sourcing .zshrc
Setting PATH from .zshrc
Sourcing .zlogin
Setting PATH from .zlogin
sourcing hook.on-activate for first time
sourcing profile.common for first time
sourcing profile.zsh for first time
Sourcing .zlogout
Setting PATH from .zlogout
EOF
}

# bats test_tags=activate,activate:zdotdir,activate:zdotdir:zprofile
@test "zsh: in-place activation from .zprofile" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  echo 'eval "$("$FLOX_BIN" activate)"' >> "$HOME/.zprofile"

  run zsh -i -l -c 'true'
  assert_success
  assert_output - <<EOF
Sourcing .zshenv
Setting PATH from .zshenv
Sourcing .zprofile
Setting PATH from .zprofile
sourcing hook.on-activate for first time
sourcing profile.common for first time
sourcing profile.zsh for first time
Sourcing .zshrc
Setting PATH from .zshrc
Sourcing .zlogin
Setting PATH from .zlogin
Sourcing .zlogout
Setting PATH from .zlogout
EOF
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:do_not_leak_FLOX_SHELL
@test "activation does not leak FLOX_SHELL variable" {
  project_setup
  FLOX_SHELL="bash" run $FLOX_BIN activate --dir "$PROJECT_DIR" -- env
  assert_success
  refute_output --partial "FLOX_SHELL="
  refute_output --partial "_flox_shell="
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:validate_hook_and_dotfile_sourcing
@test "{bash,fish,tcsh,zsh}: confirm hooks and dotfiles sourced correctly" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # This test doesn't just confirm that the right things are sourced,
  # but that they are sourced in the correct order and exactly once,
  # for all supported shells.

  echo "Testing bash"
  run bash -l -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  assert_equal "${#lines[@]}" 7
  assert_equal "${lines[0]}" "Sourcing .profile"
  assert_equal "${lines[1]}" "Setting PATH from .profile"
  assert_equal "${lines[2]}" "sourcing hook.on-activate"
  assert_equal "${lines[3]}" "Sourcing .bashrc"
  assert_equal "${lines[4]}" "Setting PATH from .bashrc"
  assert_equal "${lines[5]}" "sourcing profile.common"
  assert_equal "${lines[6]}" "sourcing profile.bash"
  echo # leave a line between test outputs

  echo "Testing fish"
  run fish -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  assert_equal "${#lines[@]}" 5
  assert_equal "${lines[0]}" "Sourcing config.fish"
  assert_equal "${lines[1]}" "Setting PATH from config.fish"
  assert_equal "${lines[2]}" "sourcing hook.on-activate"
  assert_equal "${lines[3]}" "sourcing profile.common"
  assert_equal "${lines[4]}" "sourcing profile.fish"
  echo # leave a line between test outputs

  echo "Testing tcsh"
  run tcsh -c 'eval "`$FLOX_BIN activate`"'
  assert_success
  assert_equal "${#lines[@]}" 5
  assert_equal "${lines[0]}" "Sourcing .tcshrc"
  assert_equal "${lines[1]}" "Setting PATH from .tcshrc"
  assert_equal "${lines[2]}" "sourcing hook.on-activate"
  assert_equal "${lines[3]}" "sourcing profile.common"
  assert_equal "${lines[4]}" "sourcing profile.tcsh"
  echo # leave a line between test outputs

  echo "Testing zsh"
  run zsh -i -l -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  assert_equal "${#lines[@]}" 13
  assert_equal "${lines[0]}" "Sourcing .zshenv"
  assert_equal "${lines[1]}" "Setting PATH from .zshenv"
  assert_equal "${lines[2]}" "Sourcing .zprofile"
  assert_equal "${lines[3]}" "Setting PATH from .zprofile"
  assert_equal "${lines[4]}" "Sourcing .zshrc"
  assert_equal "${lines[5]}" "Setting PATH from .zshrc"
  assert_equal "${lines[6]}" "Sourcing .zlogin"
  assert_equal "${lines[7]}" "Setting PATH from .zlogin"
  assert_equal "${lines[8]}" "sourcing hook.on-activate"
  assert_equal "${lines[9]}" "sourcing profile.common"
  assert_equal "${lines[10]}" "sourcing profile.zsh"
  assert_equal "${lines[11]}" "Sourcing .zlogout"
  assert_equal "${lines[12]}" "Setting PATH from .zlogout"
  echo # leave a line between test outputs

}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:nested_flox_activate_tracelevel
@test "{bash,fish,tcsh,zsh}: confirm _flox_activate_tracelevel set in nested activation" {
  project_setup

  # The shell-specific flox init scripts finish by unsetting the
  # _flox_activate_tracelevel environment variable, and this can
  # cause problems for an "outer" interactive activation when there
  # is an "inner" in-place activation happening by way of a "dotfile".

  # Set up this test by creating dotfiles which perform an in-place
  # activation, and then run an interactive activation of a second
  # environment to confirm that _flox_activate_tracelevel is set
  # for the outer activation.

  # Each of the shell-specific dotfiles has also been updated to emit a
  # warning if sourced without _flox_activate_tracelevel set in the
  # environment, so we also assert that this warning is not present
  # in any of the activation output.

  # Start by adding logic to create semaphore files for all shells.
  for i in "$HOME/.bashrc.extra" "$HOME/.config/fish/config.fish.extra" "$HOME/.tcshrc.extra" "$HOME/.zshrc.extra"; do
    cat >"$i" <<EOF
touch "$PROJECT_DIR/_flox_activate_tracelevel.in_test"
test -n "\$_flox_activate_tracelevel" || touch "$PROJECT_DIR/_flox_activate_tracelevel.not_defined"
EOF
  done

  # Finish by appending shell-specific flox activation syntax.
  for i in "$HOME/.bashrc.extra" "$HOME/.config/fish/config.fish.extra" "$HOME/.zshrc.extra"; do
    echo "eval \"\$($FLOX_BIN activate --dir $PROJECT_DIR)\"" >>"$i"
  done
  echo "eval \"\`$FLOX_BIN activate --dir $PROJECT_DIR\`\"" >>"$HOME/.tcshrc.extra"

  # Create a test environment.
  _temp_env="$(mktemp -d)"
  "$FLOX_BIN" init -d "$_temp_env"

  # Activate the test environment from each shell, each of which will
  # launch an interactive shell that sources the relevant dotfile.
  for target_shell in bash fish tcsh zsh; do
    echo "Testing $target_shell"
    FLOX_SHELL="$target_shell" USER="$REAL_USER" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$_temp_env"
    refute_output --partial "_flox_activate_tracelevel not defined"
    run rm "$PROJECT_DIR/_flox_activate_tracelevel.in_test"
    assert_success
    run rm "$PROJECT_DIR/_flox_activate_tracelevel.not_defined"
    assert_failure
    echo # leave a line between test outputs
  done

  rm -rf "$_temp_env"
}

# ---------------------------------------------------------------------------- #

@test "profile: RUST_SRC_PATH set when rustPlatform.rustLibSrc installed" {
  project_setup

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/rust-lib-src.json" \
    "$FLOX_BIN" install rustPlatform.rustLibSrc

  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    if ! [ -e "$FLOX_ENV/etc/profile.d/0501_rust.sh" ]; then
      echo "profile script did not exist" >&3
      exit 1
    fi
    if ! [ "$RUST_SRC_PATH" == "$FLOX_ENV" ]; then
      echo "variable was not set" >&3
      exit 1
    fi
EOF
)
  assert_success
}

@test "activate works with fish 3.2.2" {
  if [ "$NIX_SYSTEM" == aarch64-linux ]; then
    # running fish at all on aarch64-linux throws:
    # terminate called after throwing an instance of 'std::bad_alloc'
    #   what():  std::bad_alloc
    skip "fish 3.2.2 is broken on aarch64-linux"
  fi
  project_setup
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/fish_3_2_2.json" \
    "$FLOX_BIN" install fish@3.2.2

  FLOX_SHELL="./.flox/run/$NIX_SYSTEM.$PROJECT_NAME/bin/fish" run "$FLOX_BIN" activate -- echo "\$FISH_VERSION"
  assert_success
  # fish doesn't have the equivalent of set -e, so refute "Error"
  refute_output --partial Error
  assert_output --partial "3.2.2"
}

@test "no unset variables in bash" {
  project_setup
  run bash <(cat <<'EOF'
  set -u
  eval "$($FLOX_BIN activate)"
EOF
)
  refute_output --partial "_flox"
  refute_output --partial "_FLOX"
}

@test "no unset variables in zsh" {
  project_setup
  run zsh <(cat <<'EOF'
  set -u
  eval "$($FLOX_BIN activate)"
EOF
)
  refute_output --partial "_flox"
  refute_output --partial "_FLOX"
}

@test "nested interactive activate fails" {
  project_setup
  run bash <(cat <<'EOF'
    eval "$("$FLOX_BIN" activate)"

    FLOX_SHELL="bash" expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
EOF
)
  assert_failure
  assert_output --partial "is already active"
}
