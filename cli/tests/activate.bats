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
  # Make sure FLOX_BIN is set to an absolute PATH so that setting BADPATH
  # doesn't cause `flox` to be found in e.g. `/usr/local/bin`
  export FLOX_BIN="$(which "$FLOX_BIN")"
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
  # fifo is in PROJECT_DIR and keeps watchdog running,
  # so cat_teardown_fifo must be run before wait_for_watchdogs and
  # project_teardown
  cat_teardown_fifo
  # Cleaning up the `BATS_TEST_TMPDIR` occasionally fails,
  # because of an 'env-registry.json' that gets concurrently written
  # by the watchdog as the activation terminates.
  if [ -n "${PROJECT_DIR:-}" ]; then
    # Not all tests call project_setup
    wait_for_watchdogs "$PROJECT_DIR"
    project_teardown
  fi
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

HOOK_ONLY_ONCE="$(cat << "EOF"
  version = 1

  [hook]
  on-activate = """
    if [ -n "$_already_ran_hook_on_activate" ]; then
      echo "ERROR: hook section sourced twice"
      exit 1
    else
      echo "sourcing hook.on-activate for first time"
    fi
    export _already_ran_hook_on_activate=1
  """
EOF
)"

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:path,activate:path:bash
@test "bash: interactive activate puts package in path" {
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  FLOX_SHELL="bash" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:fish
@test "fish: interactive activate puts package in path" {
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  FLOX_SHELL="fish" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:tcsh
@test "tcsh: interactive activate puts package in path" {
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  FLOX_SHELL="tcsh" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
  assert_output --regexp "bin/hello"
  refute_output "not found"
}

# bats test_tags=activate,activate:path,activate:path:zsh
@test "zsh: interactive activate puts package in path" {
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install -d "$PROJECT_DIR" hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"

  FLOX_SHELL="zsh" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/interactive-hello.exp" "$PROJECT_DIR"
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
@test "bash: interactive activate runs profile scripts" {
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
}

# bats test_tags=activate,activate:hook,activate:hook:bash
@test "bash: command activate runs profile scripts" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="bash" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  assert_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:bash
@test "bash: command activate skips profile scripts with FLOX_NOPROFILE" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_NOPROFILE=1 FLOX_SHELL="bash" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:bash
@test "bash: command activate skips profile scripts with FLOX_TURBO" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_TURBO=1 FLOX_SHELL="bash" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
  FLOX_TURBO=1 FLOX_SHELL="bash" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- true
  assert_success
}

# bats test_tags=activate:standalone
@test "bash: activation script can be run directly" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly in various forms.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="bash" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.run/activate -c :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  assert_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="bash" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --command :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="bash" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -c true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="bash" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --command true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="bash" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="bash" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -- :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="bash" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="bash" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -- true
  assert_success
}

# bats test_tags=activate
@test "bash: activation script can be run with --noprofile" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly with --noprofile.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="bash" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --noprofile :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate
@test "bash: activation script can be run with --turbo" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly with --turbo.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="bash" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --turbo :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:fish
@test "fish: interactive activate runs profile scripts" {
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
}

# bats test_tags=activate,activate:hook,activate:hook:fish
@test "fish: command activate runs profile scripts" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="fish" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  assert_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:fish
@test "fish: command activate skips profile scripts with FLOX_NOPROFILE" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_NOPROFILE=1 FLOX_SHELL="fish" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}


# bats test_tags=activate,activate:hook,activate:hook:fish
@test "fish: command activate skips profile scripts with FLOX_TURBO" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_TURBO=1 FLOX_SHELL="fish" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
  FLOX_TURBO=1 FLOX_SHELL="fish" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- true
  assert_success
}

# bats test_tags=activate
@test "fish: activation script can be run directly" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly in various forms.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="fish" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -c :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  assert_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="fish" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --command :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="fish" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -c true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="fish" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --command true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="fish" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="fish" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -- :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="fish" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="fish" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -- true
  assert_success
}

# bats test_tags=activate
@test "fish: activation script can be run directly with --noprofile" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly with --noprofile.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="fish" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --noprofile :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate
@test "fish: activation script can be run directly with --turbo" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly with --turbo.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="fish" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --turbo :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:tcsh
@test "tcsh: interactive activate runs profile scripts" {
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
}

# bats test_tags=activate,activate:hook,activate:hook:tcsh
@test "tcsh: command activate runs profile scripts" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="tcsh" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  assert_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:tcsh
@test "tcsh: command activate skips profile scripts with FLOX_NOPROFILE" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_NOPROFILE=1 FLOX_SHELL="tcsh" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:tcsh
@test "tcsh: command activate skips profile scripts with FLOX_TURBO" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_TURBO=1 FLOX_SHELL="tcsh" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
  FLOX_TURBO=1 FLOX_SHELL="tcsh" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- true
  assert_success
}

# bats test_tags=activate
@test "tcsh: activation script can be run directly" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly in various forms.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="tcsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -c :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  assert_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="tcsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --command :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="tcsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -c true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="tcsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --command true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="tcsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="tcsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -- :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="tcsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="tcsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -- true
  assert_success
}

# bats test_tags=activate
@test "tcsh: activation script can be run directly with --noprofile" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly with --noprofile.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="tcsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --noprofile :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate
@test "tcsh: activation script can be run directly with --turbo" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly with --turbo.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="tcsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --turbo :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:zsh
@test "zsh: interactive activate runs profile scripts" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"


  # FLOX_SHELL="zsh" run -0 bash -c "echo exit | $FLOX_CLI activate --dir $PROJECT_DIR";
  FLOX_SHELL="zsh" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  assert_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:zsh
@test "zsh: command activate runs profile scripts" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="zsh" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  assert_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:zsh
@test "zsh: command activate skips profile scripts with FLOX_NOPROFILE" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_NOPROFILE=1 FLOX_SHELL="zsh" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate,activate:hook,activate:hook:zsh
@test "zsh: command activate skips profile scripts with FLOX_TURBO" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_TURBO=1 FLOX_SHELL="zsh" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
  FLOX_TURBO=1 FLOX_SHELL="zsh" NO_COLOR=1 run $FLOX_BIN activate --dir "$PROJECT_DIR" -- true
  assert_success
}

# bats test_tags=activate
@test "zsh: activation script can be run directly" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly in various forms.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="zsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -c :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  assert_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  assert_output --partial "sourcing profile.zsh"
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="zsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --command :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="zsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -c true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="zsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --command true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="zsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="zsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -- :
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="zsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate true
  assert_success
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="zsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate -- true
  assert_success
}

# bats test_tags=activate
@test "zsh: activation script can be run directly with --noprofile" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly with --noprofile.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="zsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --noprofile :
  assert_success
  assert_output --partial "sourcing hook.on-activate"
  refute_output --partial "sourcing profile.common"
  refute_output --partial "sourcing profile.bash"
  refute_output --partial "sourcing profile.fish"
  refute_output --partial "sourcing profile.tcsh"
  refute_output --partial "sourcing profile.zsh"
}

# bats test_tags=activate
@test "zsh: activation script can be run directly with --turbo" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml" | "$FLOX_BIN" edit -f -

  # Test running the activate script directly with --turbo.
  FLOX_RUNTIME_DIR="$FLOX_CACHE_DIR" FLOX_SHELL="zsh" NO_COLOR=1 run $PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/activate --turbo :
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
  cat <<'EOF' | bash
    output="$(FLOX_SHELL="bash" eval "$("$FLOX_BIN" activate)")"
    [[ "$output" == *"sourcing profile.bash"* ]]
    output="$(FLOX_SHELL="bash" eval "$("$FLOX_BIN" activate)")"
    [[ "$output" == *"sourcing profile.bash"* ]]
EOF
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
@test "bash: activate command-mode runs hook and profile scripts only once" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  FLOX_SHELL="bash" NO_COLOR=1 run "$FLOX_BIN" activate -- :
  assert_success
  refute_output --partial "ERROR"
  assert_output --partial "sourcing hook.on-activate for first time"
  assert_output --partial "sourcing profile.bash for first time"
  refute_output --partial "sourcing profile.zsh for first time"
}

# bats test_tags=activate,activate:once
@test "bash: interactive activate runs hook and profile scripts only once" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  FLOX_SHELL="bash" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
  assert_success
  refute_output --partial "ERROR"
  assert_output --partial "sourcing hook.on-activate for first time"
  assert_output --partial "sourcing profile.bash for first time"
  refute_output --partial "sourcing profile.zsh for first time"
}

# bats test_tags=activate,activate:once
@test "zsh: activate command-mode runs hook and profile scripts only once" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  FLOX_SHELL="zsh" NO_COLOR=1 run "$FLOX_BIN" activate -- :
  assert_success
  refute_output --partial "ERROR"
  assert_output --partial "sourcing hook.on-activate for first time"
  refute_output --partial "sourcing profile.bash for first time"
  assert_output --partial "sourcing profile.zsh for first time"
}

# bats test_tags=activate,activate:once
@test "zsh: interactive activate runs hook and profile scripts only once" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/only-once.toml"

  FLOX_SHELL="zsh" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
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

  FLOX_SHELL="bash" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
  assert_output --partial "test_alias is aliased to \`echo testing'"
}

# bats test_tags=activate,activate:fish,activate:rc:fish
@test "fish: activate respects ~/.config/fish/config.fish" {
  project_setup
  echo "alias test_alias='echo testing'" >"$HOME/.config/fish/config.fish.extra"

  FLOX_SHELL="fish" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
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

  FLOX_SHELL="tcsh" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc-tcsh.exp" "$PROJECT_DIR"
  assert_line --partial "echo testing"
}

# bats test_tags=activate,activate:rc:zsh
@test "zsh: activate respects ~/.zshrc" {
  project_setup
  echo "alias test_alias='echo testing'" >"$HOME/.zshrc.extra"

  FLOX_SHELL="zsh" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/rc.exp" "$PROJECT_DIR"
  assert_output --partial "test_alias is an alias for echo testing"
}

# bats test_tags=activate,activate:rc:zsh
@test "zsh: interactive activate respects history settings from dotfile" {
  project_setup

  # This should always work, even when Darwin sets a default in `/etc/zshrc`.
  echo 'HISTFILE=${PROJECT_DIR}/.alt_history' >"$HOME/.zshrc.extra"
  echo 'SHELL_SESSION_DIR=${PROJECT_DIR}/.alt_sessions' >>"$HOME/.zshrc.extra"


  FLOX_SHELL="zsh" NO_COLOR=1 \
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


  FLOX_SHELL="zsh" NO_COLOR=1 \
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


  FLOX_SHELL="zsh" NO_COLOR=1 \
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


  FLOX_SHELL="zsh" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/envVar.exp" "$PROJECT_DIR"
  assert_output --partial "baz"

  FLOX_SHELL="zsh" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- echo '$foo'
  assert_success
  assert_output --partial "baz"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:envVar-before-hook
@test "bash: activate sets env var before hook" {
  project_setup
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT_ECHO_FOO//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"


  FLOX_SHELL="bash" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
}

# bats test_tags=activate,activate:envVar-before-hook
@test "fish: activate sets env var before hook" {
  project_setup
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT_ECHO_FOO//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="fish" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
}


# bats test_tags=activate,activate:envVar-before-hook
@test "tcsh: activate sets env var before hook" {
  project_setup
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT_ECHO_FOO//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="tcsh" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
}


# bats test_tags=activate,activate:envVar-before-hook
@test "zsh: activate sets env var before hook" {
  project_setup
  sed -i -e "s/^\[vars\]/${VARS//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT_ECHO_FOO//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  FLOX_SHELL="zsh" NO_COLOR=1 run "$FLOX_BIN" activate --dir "$PROJECT_DIR" -- exit
  assert_success
  assert_output --partial "baz"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:path,activate:path:bash
@test "'flox activate' modifies path (bash)" {
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
  assert_line --partial "set-prompt.bash"
}

# bats test_tags=activate,activate:inplace-prints
@test "'flox activate' prints script to modify current shell (fish)" {
  project_setup
  # Flox detects that the output is not a tty and prints the script to stdout
  FLOX_SHELL="fish" run "$FLOX_BIN" activate
  assert_success
  # check that env vars are set for compatibility with nix built software
  assert_line --partial "set -gx NIX_SSL_CERT_FILE "
  assert_line --partial "set-prompt.fish"
}

# bats test_tags=activate,activate:inplace-prints
@test "'flox activate' prints script to modify current shell (tcsh)" {
  project_setup
  # Flox detects that the output is not a tty and prints the script to stdout
  FLOX_SHELL="tcsh" run "$FLOX_BIN" activate
  assert_success
  # check that env vars are set for compatibility with nix built software
  assert_line --partial "setenv NIX_SSL_CERT_FILE "
  assert_line --partial "set-prompt.tcsh"
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

# bats test_tags=activate,activate:python-detects-installed-python
@test "'flox activate' sets python vars if python is installed" {
  project_setup
  # unset python vars if any
  unset PYTHONPATH
  unset PIP_CONFIG_FILE

  # install python and pip
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/python311Packages.pip.json"
  "$FLOX_BIN" install python311Packages.pip

  run -- "$FLOX_BIN" activate -- echo PYTHONPATH is '$PYTHONPATH'
  assert_success
  assert_line "PYTHONPATH is $(realpath $PROJECT_DIR)/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/lib/python3.11/site-packages"

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
@test "'flox *' uses local environment over 'default' environment" {
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

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:bash
@test "'hook.on-activate' modifies environment variables for first nested activation (bash)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"

  cat <<'EOF' | bash
    eval "$("$FLOX_BIN" activate)"
    if [[ "$foo" != "baz" ]]; then
      echo "foo=$foo when it should be foo=baz"
      exit 1
    fi
    unset foo
    eval "$("$FLOX_BIN" activate)"
    if [[ ! -z "${foo:-}" ]]; then
      echo "foo=$foo when it should be unset"
      exit 1
    fi
EOF
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:fish
@test "'hook.on-activate' modifies environment variables for first nested activation (fish)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"

  cat <<'EOF' | fish
    eval "$("$FLOX_BIN" activate)"
    echo "$foo" | string match "baz"
    set -e foo
    eval "$("$FLOX_BIN" activate)"
    if set -q foo
      echo "foo=$foo when it should be unset"
      exit 1
    end
EOF
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:tcsh
@test "'hook.on-activate' modifies environment variables for first nested activation (tcsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"

  cat <<'EOF' | tcsh -v
    eval "`$FLOX_BIN activate`"
    if ( "$foo" != "baz" ) then
      echo "foo=$foo when it should be foo=baz"
      exit 1
    endif
    unsetenv foo
    eval "`$FLOX_BIN activate`"
    if ( $?foo ) then
      echo "foo=$foo when it should be unset"
      exit 1
    endif
EOF
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:zsh
@test "'hook.on-activate' modifies environment variables for first nested activation (zsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/on-activate.toml"

  cat <<'EOF' | zsh
    eval "$("$FLOX_BIN" activate)"
    if [[ "$foo" != "baz" ]]; then
      echo "foo=$foo when it should be foo=baz"
      exit 1
    fi
    unset foo
    eval "$("$FLOX_BIN" activate)"
    if [[ ! -z "${foo:-}" ]]; then
      echo "foo=$foo when it should be unset"
      exit 1
    fi
EOF
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:bash
@test "'hook.on-activate' unsets environment variables for first nested activation (bash)" {
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

  cat <<'EOF' | bash
    export foo=baz
    eval "$(FLOX_SHELL="bash" "$FLOX_BIN" activate)"
    if [[ ! -z "${foo:-}" ]]; then
      echo "foo=$foo when it should be unset"
      exit 1
    fi
    export foo=baz
    eval "$(FLOX_SHELL="bash" "$FLOX_BIN" activate)"
    if [[ "$foo" != "baz" ]]; then
      echo "foo=$foo when it should be foo=baz"
      exit 1
    fi
EOF
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:fish
@test "'hook.on-activate' unsets environment variables for first nested activation (fish)" {
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

  cat <<'EOF' | fish
    set -gx foo baz
    eval "$("$FLOX_BIN" activate)"
    if set -q foo
      echo "foo=$foo when it should be unset"
      exit 1
    end
    set -gx foo baz
    eval "$("$FLOX_BIN" activate)"
    echo "$foo" | string match "baz"
EOF
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:tcsh
@test "'hook.on-activate' unsets environment variables for first nested activation (tcsh)" {
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

  cat <<'EOF' | tcsh
    setenv foo baz
    eval "`$FLOX_BIN activate`"
    if ( $?foo ) then
      echo "foo=$foo when it should be unset"
      exit 1
    endif
    setenv foo baz
    eval "`$FLOX_BIN activate`"
    if ( "$foo" != "baz" ) then
      echo "foo=$foo when it should be foo=baz"
      exit 1
    endif
EOF
}

# bats test_tags=activate:scripts:on-activate,activate:scripts:on-activate:zsh
@test "'hook.on-activate' unsets environment variables for first nested activation (zsh)" {
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

  cat <<'EOF' | zsh
    export foo=baz
    eval "$("$FLOX_BIN" activate)"
    if [[ ! -z "${foo:-}" ]]; then
      echo "foo=$foo when it should be unset"
      exit 1
    fi
    export foo=baz
    eval "$("$FLOX_BIN" activate)"
    if [[ "$foo" != "baz" ]]; then
      echo "foo=$foo when it should be foo=baz"
      exit 1
    fi
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
  FLOX_SHELL=zsh NO_COLOR=1 run zsh --interactive --login -c \
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
@test "bash: confirm hooks and dotfiles sourced correctly" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # Prevent `/etc/bashrc_Apple_Terminal` from altering output with:
  #   Saving session...completed.
  #   Deleting expired sessions...none found.
  if [[ "$NIX_SYSTEM" == *"-darwin" ]]; then
    touch "${HOME}/.bash_sessions_disable"
  fi

  # This test doesn't just confirm that the right things are sourced,
  # but that they are sourced in the correct order and exactly once.

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
}

# bats test_tags=activate,activate:validate_hook_and_dotfile_sourcing
@test "fish: confirm hooks and dotfiles sourced correctly" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # This test doesn't just confirm that the right things are sourced,
  # but that they are sourced in the correct order and exactly once.

  run fish -c 'eval "$("$FLOX_BIN" activate)"'
  assert_success
  assert_equal "${#lines[@]}" 5
  assert_equal "${lines[0]}" "Sourcing config.fish"
  assert_equal "${lines[1]}" "Setting PATH from config.fish"
  assert_equal "${lines[2]}" "sourcing hook.on-activate"
  assert_equal "${lines[3]}" "sourcing profile.common"
  assert_equal "${lines[4]}" "sourcing profile.fish"
}

# bats test_tags=activate,activate:validate_hook_and_dotfile_sourcing
@test "tcsh: confirm hooks and dotfiles sourced correctly" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # This test doesn't just confirm that the right things are sourced,
  # but that they are sourced in the correct order and exactly once.

  run tcsh -c 'eval "`$FLOX_BIN activate`"'
  assert_success
  assert_equal "${#lines[@]}" 5
  assert_equal "${lines[0]}" "Sourcing .tcshrc"
  assert_equal "${lines[1]}" "Setting PATH from .tcshrc"
  assert_equal "${lines[2]}" "sourcing hook.on-activate"
  assert_equal "${lines[3]}" "sourcing profile.common"
  assert_equal "${lines[4]}" "sourcing profile.tcsh"
}

# bats test_tags=activate,activate:validate_hook_and_dotfile_sourcing
@test "zsh: confirm hooks and dotfiles sourced correctly" {
  project_setup
  sed -i -e "s/^\[profile\]/${HELLO_PROFILE_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"
  sed -i -e "s/^\[hook\]/${VARS_HOOK_SCRIPT//$'\n'/\\n}/" "$PROJECT_DIR/.flox/env/manifest.toml"

  # This test doesn't just confirm that the right things are sourced,
  # but that they are sourced in the correct order and exactly once.

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
}

# ---------------------------------------------------------------------------- #

# test function run for each shell to confirm _flox_activate_tracelevel set in
# nested activation
confirm_tracelevel() {
  shell="${1?}"
  # dotfile that performs an in-place activation, see longer description below
  extra_config_path="${2?}"
  extra_config_content="${3?}"

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

  # Start by adding logic to create a semaphore file
  echo "$extra_config_content" > "$extra_config_path"

  # Activate the test environment, which will launch an interactive shell that
  # sources the relevant dotfile.

  FLOX_SHELL="$shell" NO_COLOR=1 run -0 expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
  refute_output --partial "_flox_activate_tracelevel not defined"
  run rm "$PROJECT_DIR/_flox_activate_tracelevel.in_test"
  assert_success
  run rm "$PROJECT_DIR/_flox_activate_tracelevel.not_defined"
  assert_failure
}

# bats test_tags=activate,activate:nested_flox_activate_tracelevel
@test "bash: confirm _flox_activate_tracelevel set in nested activation" {
  project_setup

  bashrc_content="$(cat <<EOF
touch "$PROJECT_DIR/_flox_activate_tracelevel.in_test"
test -n "\$_flox_activate_tracelevel" || touch "$PROJECT_DIR/_flox_activate_tracelevel.not_defined"
eval "\$($FLOX_BIN activate --dir $PROJECT_DIR)"
EOF
  )"

  confirm_tracelevel bash "$HOME/.bashrc.extra" "$bashrc_content"
}

# bats test_tags=activate,activate:nested_flox_activate_tracelevel
@test "fish: confirm _flox_activate_tracelevel set in nested activation" {
  project_setup

  config_fish_content="$(cat <<EOF
touch "$PROJECT_DIR/_flox_activate_tracelevel.in_test"
test -n "\$_flox_activate_tracelevel" || touch "$PROJECT_DIR/_flox_activate_tracelevel.not_defined"
eval "\$($FLOX_BIN activate --dir $PROJECT_DIR)"
EOF
  )"

  confirm_tracelevel fish "$HOME/.config/fish/config.fish.extra" "$config_fish_content"
}

# bats test_tags=activate,activate:nested_flox_activate_tracelevel
@test "tcsh: confirm _flox_activate_tracelevel set in nested activation" {
  project_setup

  tcshrc_content="$(cat <<EOF
touch "$PROJECT_DIR/_flox_activate_tracelevel.in_test"
test -n "\$_flox_activate_tracelevel" || touch "$PROJECT_DIR/_flox_activate_tracelevel.not_defined"
eval "\`$FLOX_BIN activate --dir $PROJECT_DIR\`"
EOF
  )"

  confirm_tracelevel tcsh "$HOME/.tcshrc.extra" "$tcshrc_content"
}

# bats test_tags=activate,activate:nested_flox_activate_tracelevel
@test "zsh: confirm _flox_activate_tracelevel set in nested activation" {
  project_setup

  zshrc_content="$(cat <<EOF
touch "$PROJECT_DIR/_flox_activate_tracelevel.in_test"
test -n "\$_flox_activate_tracelevel" || touch "$PROJECT_DIR/_flox_activate_tracelevel.not_defined"
eval "\$($FLOX_BIN activate --dir $PROJECT_DIR)"
EOF
  )"

  confirm_tracelevel zsh "$HOME/.zshrc.extra" "$zshrc_content"
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

@test "profile: JUPYTER_PATH not modified when Jupyter is not installed" {
  project_setup

  # Shouldn't be set by default.
  run --separate-stderr "$FLOX_BIN" activate -- \
    bash -c 'echo "JUPYTER_PATH is: $JUPYTER_PATH"'
  assert_success
  assert_output "JUPYTER_PATH is: "

  # Should respect existing variable from outside activation.
  JUPYTER_PATH="/fakepath" run --separate-stderr "$FLOX_BIN" activate -- \
    bash -c 'echo "JUPYTER_PATH is: $JUPYTER_PATH"'
  assert_success
  assert_output "JUPYTER_PATH is: /fakepath"
}

@test "profile: JUPYTER_PATH is modified when Jupyter is installed" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  # Mock contains both extensions but only one is used in each install.
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/jupyter_with_extensions.json"

  PACKAGES_OUTER="jupyter python312Packages.jupyterlab-widgets"
  PACKAGES_INNER="jupyter python312Packages.jupyterlab-git"

  EXPECTED_NOTEBOOK="@jupyter-notebook/lab-extension"
  EXPECTED_WIDGETS="@jupyter-widgets/jupyterlab-manager"
  EXPECTED_GIT="@jupyterlab/git"

  # Test outer project by itself.
  "$FLOX_BIN" init --dir=outer
  run "$FLOX_BIN" install --dir=outer $PACKAGES_OUTER
  assert_success

  run "$FLOX_BIN" activate --dir=outer -- \
    jupyter labextension list
  assert_success
  assert_line --partial "$EXPECTED_NOTEBOOK" # from outer
  assert_line --partial "$EXPECTED_WIDGETS"  # from outer
  refute_line --partial "$EXPECTED_GIT"      # not from inner

  # Test outer and inner project combined.
  "$FLOX_BIN" init --dir=inner
  run "$FLOX_BIN" install --dir=inner $PACKAGES_INNER
  assert_success

  run "$FLOX_BIN" activate --dir=outer -- \
    "$FLOX_BIN" activate --dir=inner -- \
    jupyter labextension list
  assert_success
  assert_line --partial "$EXPECTED_NOTEBOOK" # from either
  assert_line --partial "$EXPECTED_WIDGETS"  # from outer
  assert_line --partial "$EXPECTED_GIT"      # from inner
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

  FLOX_SHELL="./.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/bin/fish" run "$FLOX_BIN" activate -- echo "\$FISH_VERSION"
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

    expect "$TESTS_DIR/activate/activate.exp" "$PROJECT_DIR"
EOF
)
  assert_failure
  assert_output --partial "Environment '$PROJECT_NAME' is already active"
}

# ---------------------------------------------------------------------------- #
# Test that attach does not run hooks a second time after they've already been
# run by the initial activation
# Run test for each of 3 activation modes
# Don't test for every shell since hooks are run in our activation scripts
# before starting the user's shell
# ---------------------------------------------------------------------------- #

attach_runs_hooks_once() {
  mode="${1?}"

  echo "$HOOK_ONLY_ONCE" | "$FLOX_BIN" edit -f -

  mkfifo activate_finished
  # Will get cat'ed in teardown
  TEARDOWN_FIFO="$PROJECT_DIR/teardown_activate"
  mkfifo "$TEARDOWN_FIFO"

  "$FLOX_BIN" activate -- bash -c "echo > activate_finished && echo > \"$TEARDOWN_FIFO\"" 2> output &

  cat activate_finished
  run cat output
  assert_output --partial "sourcing hook.on-activate for first time"
  assert_output --partial "hook.on-activate"

  case "$mode" in
    interactive)
      NO_COLOR=1 run expect "$TESTS_DIR/activate/attach.exp" "$PROJECT_DIR" true
      ;;
    command)
      run "$FLOX_BIN" activate -- true
      ;;
    in-place)
      run bash -c 'eval "$("$FLOX_BIN" activate)"'
      ;;
  esac
  assert_success

  refute_output --partial "sourcing hook.on-activate for first time"
}

# bats test_tags=activate,activate:attach
@test "interactive: attach runs hook once" {
  project_setup
  attach_runs_hooks_once interactive
}

# bats test_tags=activate,activate:attach
@test "command-mode: attach runs hook once" {
  project_setup
  attach_runs_hooks_once command
}

# bats test_tags=activate,activate:attach
@test "in-place: attach runs hook once" {
  project_setup
  attach_runs_hooks_once in-place
}

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #
# Test that attach runs profile scripts even though they have already been run
# by the initial activation
# Run test for 4 shells in each of 3 modes
# ---------------------------------------------------------------------------- #

attach_runs_profile_twice() {
  shell="${1?}"
  mode="${2?}"

  "$FLOX_BIN" edit -f "$TESTS_DIR/activate/attach_runs_profile_twice.toml"

  mkfifo activate_finished
  # Will get cat'ed in teardown
  TEARDOWN_FIFO="$PROJECT_DIR/teardown_activate"
  mkfifo "$TEARDOWN_FIFO"

  # Our tcsh quoting appears to be broken so don't quote $TEARDOWN_FIFO
  FLOX_SHELL="$shell" "$FLOX_BIN" activate -- bash -c "echo > activate_finished && echo > $TEARDOWN_FIFO" >> output 2>&1 &

  cat activate_finished
  run cat output
  assert_output --partial "sourcing profile.common"
  assert_output --partial "sourcing profile.$shell"

  case "$mode" in
    interactive)
      FLOX_SHELL="$shell" NO_COLOR=1 run expect "$TESTS_DIR/activate/attach.exp" "$PROJECT_DIR" true
      ;;
    command)
      FLOX_SHELL="$shell" run "$FLOX_BIN" activate -- true
      ;;
    in-place)
      if [ "$shell" == "tcsh" ]; then
        run "$shell" -c 'eval "`"$FLOX_BIN" activate`"'
      else
        run "$shell" -c 'eval "$("$FLOX_BIN" activate)"'
      fi
      ;;
  esac

  assert_success
  assert_output --partial "sourcing profile.common"
  assert_output --partial "sourcing profile.$shell"
}

# bats test_tags=activate,activate:attach
@test "bash: interactive: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice bash interactive
}

# bats test_tags=activate,activate:attach
@test "bash: command-mode: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice bash command
}

# bats test_tags=activate,activate:attach
@test "bash: in-place: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice bash in-place
}

# bats test_tags=activate,activate:attach
@test "fish: interactive: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice fish interactive
}

# bats test_tags=activate,activate:attach
@test "fish: command-mode: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice fish command
}

# bats test_tags=activate,activate:attach
@test "fish: in-place: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice fish in-place
}

# bats test_tags=activate,activate:attach
@test "tcsh: interactive: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice tcsh interactive
}

# bats test_tags=activate,activate:attach
@test "tcsh: command-mode: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice tcsh command
}

# bats test_tags=activate,activate:attach
@test "tcsh: in-place: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice tcsh in-place
}

# bats test_tags=activate,activate:attach
@test "zsh: interactive: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice zsh interactive
}

# bats test_tags=activate,activate:attach
@test "zsh: command-mode: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice zsh command
}

# bats test_tags=activate,activate:attach
@test "zsh: in-place: attach runs profile twice" {
  project_setup
  attach_runs_profile_twice zsh in-place
}

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #
# Test that attach sets vars exported in hooks
# Run test for 4 shells in each of 3 modes
# ---------------------------------------------------------------------------- #

attach_sets_hook_vars() {
  shell="${1?}"
  mode="${2?}"

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [hook]
    on-activate = """
      export HOOK_ON_ACTIVATE="hook.on-activate var"
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  mkfifo activate_finished
  # Will get cat'ed in teardown
  TEARDOWN_FIFO="$PROJECT_DIR/teardown_activate"
  mkfifo "$TEARDOWN_FIFO"

  # Our tcsh quoting appears to be broken so don't quote $TEARDOWN_FIFO
  FLOX_SHELL="$shell" "$FLOX_BIN" activate -- bash -c "echo > activate_finished && echo > $TEARDOWN_FIFO" >> output 2>&1 &

  cat activate_finished

  case "$mode" in
    interactive)
      FLOX_SHELL="$shell" NO_COLOR=1 run expect "$TESTS_DIR/activate/attach.exp" "$PROJECT_DIR" "echo \$HOOK_ON_ACTIVATE"
      ;;
    command)
      FLOX_SHELL="$shell" run "$FLOX_BIN" activate -- echo \$HOOK_ON_ACTIVATE
      ;;
    in-place)
      if [ "$shell" == "tcsh" ]; then
        run "$shell" -c 'eval "`"$FLOX_BIN" activate`" && echo "$HOOK_ON_ACTIVATE"'
      else
        run "$shell" -c 'eval "$("$FLOX_BIN" activate)" && echo "$HOOK_ON_ACTIVATE"'
      fi
      ;;
  esac

  assert_success
  assert_output --partial "hook.on-activate var"
}

# bats test_tags=activate,activate:attach
@test "bash: interactive: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars bash interactive
}

# bats test_tags=activate,activate:attach
@test "bash: command-mode: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars bash command
}

# bats test_tags=activate,activate:attach
@test "bash: in-place: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars bash in-place
}

# bats test_tags=activate,activate:attach
@test "fish: interactive: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars fish interactive
}

# bats test_tags=activate,activate:attach
@test "fish: command-mode: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars fish command
}

# bats test_tags=activate,activate:attach
@test "fish: in-place: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars fish in-place
}

# bats test_tags=activate,activate:attach
@test "tcsh: interactive: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars tcsh interactive
}

# bats test_tags=activate,activate:attach
@test "tcsh: command-mode: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars tcsh command
}

# bats test_tags=activate,activate:attach
@test "tcsh: in-place: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars tcsh in-place
}

# bats test_tags=activate,activate:attach
@test "zsh: interactive: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars zsh interactive
}

# bats test_tags=activate,activate:attach
@test "zsh: command-mode: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars zsh command
}

# bats test_tags=activate,activate:attach
@test "zsh: in-place: attach sets vars from hook" {
  project_setup
  attach_sets_hook_vars zsh in-place
}

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #
# Test that attach sets vars set in profile scripts
# Run test for 4 shells in each of 3 modes
# ---------------------------------------------------------------------------- #

attach_sets_profile_vars() {
  shell="${1?}"
  mode="${2?}"
  MANIFEST_CONTENTS="${3?}"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  mkfifo activate_finished
  # Will get cat'ed in teardown
  TEARDOWN_FIFO="$PROJECT_DIR/teardown_activate"
  mkfifo "$TEARDOWN_FIFO"

  # Our tcsh quoting appears to be broken so don't quote $TEARDOWN_FIFO
  FLOX_SHELL="$shell" "$FLOX_BIN" activate -- bash -c "echo > activate_finished && echo > $TEARDOWN_FIFO" &

  cat activate_finished

  case "$mode" in
    interactive)
      # using assert_line with expect is racey so just direct the output we need to a file
      FLOX_SHELL="$shell" NO_COLOR=1 expect "$TESTS_DIR/activate/attach.exp" "$PROJECT_DIR" "echo \$PROFILE_COMMON > output && echo \$PROFILE_$shell >> output"
      run cat output
      ;;
    command)
      FLOX_SHELL="$shell" run "$FLOX_BIN" activate -- echo \$PROFILE_COMMON \&\& echo "\$PROFILE_$shell"
      ;;
    in-place)
      if [ "$shell" == "tcsh" ]; then
        # Single quote what we don't want expanded
        # Double quote $shell
        run "$shell" -c 'eval "`"$FLOX_BIN" activate`" && echo $PROFILE_COMMON && echo $PROFILE_'"$shell"
      else
        # Single quote what we don't want expanded
        # Double quote $shell
        run "$shell" -c 'eval "$("$FLOX_BIN" activate)" && echo $PROFILE_COMMON && echo $PROFILE_'"$shell"
      fi
      ;;
  esac

  assert_success
  # use assert_line rather than --partial since fish will print errors like
  # Unsupported use of '='. In fish, please use 'set PROFILE_COMMON "profile.common var"'.
  assert_line "profile.common var"
  assert_line "profile.$shell var"
}

BASH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS="$(cat << "EOF"
  version = 1

  [profile]
  common = """
    PROFILE_COMMON="profile.common var"
  """
  bash = """
    PROFILE_bash="profile.bash var"
  """
EOF
)"

# bats test_tags=activate,activate:attach
@test "bash: interactive: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars bash interactive "$BASH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

# bats test_tags=activate,activate:attach
@test "bash: command-mode: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars bash command "$BASH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

# bats test_tags=activate,activate:attach
@test "bash: in-place: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars bash in-place "$BASH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

FISH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS="$(cat << "EOF"
  version = 1

  [profile]
  common = """
    set PROFILE_COMMON "profile.common var"
  """
  fish = """
    set PROFILE_fish "profile.fish var"
  """
EOF
)"

# bats test_tags=activate,activate:attach
@test "fish: interactive: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars fish interactive "$FISH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

# bats test_tags=activate,activate:attach
@test "fish: command-mode: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars fish command "$FISH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

# bats test_tags=activate,activate:attach
@test "fish: in-place: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars fish in-place "$FISH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

TCSH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS="$(cat << "EOF"
  version = 1

  [profile]
  common = """
    set PROFILE_COMMON="profile.common var"
  """
  tcsh = """
    set PROFILE_tcsh="profile.tcsh var"
  """
EOF
)"

# bats test_tags=activate,activate:attach
@test "tcsh: interactive: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars tcsh interactive "$TCSH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

# bats test_tags=activate,activate:attach
@test "tcsh: command-mode: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars tcsh command "$TCSH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

# bats test_tags=activate,activate:attach
@test "tcsh: in-place: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars tcsh in-place "$TCSH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

ZSH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS="$(cat << "EOF"
  version = 1

  [profile]
  common = """
    PROFILE_COMMON="profile.common var"
  """
  zsh = """
    PROFILE_zsh="profile.zsh var"
  """
EOF
)"

# bats test_tags=activate,activate:attach
@test "zsh: interactive: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars zsh interactive "$ZSH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

# bats test_tags=activate,activate:attach
@test "zsh: command-mode: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars zsh command "$ZSH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

# bats test_tags=activate,activate:attach
@test "zsh: in-place: attach sets vars from profile" {
  project_setup
  attach_sets_profile_vars zsh in-place "$ZSH_ATTACH_SETS_PROFILE_VARS_MANIFEST_CONTENTS"
}

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #
# Test that attach sets vars set in profile scripts
# Run test for 4 shells in each of 3 modes
# ---------------------------------------------------------------------------- #

activation_gets_cleaned_up() {
  mode="${1?}"

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [hook]
    on-activate = """
      export FOO="$injected"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  mkfifo activate_finished
  # Will get cat'ed in teardown
  TEARDOWN_FIFO="$PROJECT_DIR/teardown_activate"
  mkfifo "$TEARDOWN_FIFO"

  # Start a first_activation which sets FOO=first_activation
  case "$mode" in
    command)
      injected="first_activation" _FLOX_WATCHDOG_LOG_LEVEL=trace "$FLOX_BIN" activate -- bash -c "echo \$FOO > output && echo > activate_finished && echo > $TEARDOWN_FIFO" &
      ;;
    in-place)
      TEARDOWN_FIFO="$TEARDOWN_FIFO" injected="first_activation" bash -c 'eval "$(_FLOX_WATCHDOG_LOG_LEVEL=trace "$FLOX_BIN" activate)" && echo $FOO > output && echo > activate_finished && echo > "$TEARDOWN_FIFO"' &
      ;;
  esac

  timeout 2 cat activate_finished

  run cat output
  assert_success
  assert_output "first_activation"

  # Wait for the watchdog to poll at least once so the test doesn't pass just
  # because the 2nd activate beats the watchdog to poll

  # First wait for the logfile to appear
  timeout 1s bash -c '
    while ! ls $PROJECT_DIR/.flox/log/watchdog.*.log.*; do
      sleep .1
    done
  '
  watchdog_1_log="$(echo $PROJECT_DIR/.flox/log/watchdog.*.log.*)"
  initial_number_of_polls="$(cat "$watchdog_1_log" | grep "still watching PIDs" | wc -l)"
  watchdog_1_log="$watchdog_1_log" initial_number_of_polls="$initial_number_of_polls" \
    timeout 1s bash -c '
      while [ "$(cat "$watchdog_1_log" | grep "still watching PIDs" | wc -l)" == "$initial_number_of_polls" ]; do
        sleep .1
      done
    '

  # Run a second activation which should attach to the first,
  # so FOO should still be first_activation
  injected="second_activation" run --separate-stderr "$FLOX_BIN" activate -- echo \$FOO
  assert_success
  assert_output "first_activation"

  # Teardown the first activation and wait for the watchdog to clean it up
  cat "$TEARDOWN_FIFO"
  unset TEARDOWN_FIFO # otherwise teardown will hang
  wait_for_watchdogs "$PROJECT_DIR"

  # Verify that a third activation starts rather than attaching
  injected="third_activation" run  "$FLOX_BIN" activate -- echo \$FOO
  assert_success
  assert_output --partial "third_activation"
}

# bats test_tags=activate,activate:attach
@test "command-mode: activation gets cleaned up and subsequent activation starts" {
  project_setup
  activation_gets_cleaned_up command
}

# bats test_tags=activate,activate:attach
@test "in-place: activation gets cleaned up and subsequent activation starts" {
  project_setup
  activation_gets_cleaned_up in-place
}

# ---------------------------------------------------------------------------- #

# bats test_tags=activate,activate:attach
# NB: There is a corresponding test in `services.bats`.
@test "version: refuses to attach to an older activations.json version" {
  project_setup

  # TODO: Workaround for https://github.com/flox/flox/issues/2164
  rm "${HOME}/.bashrc"

  # Prevent backtraces from `flox-activations` leaking into output.
  unset RUST_BACKTRACE

  export -f jq_edit
  run "$FLOX_BIN" activate -- bash <(
    cat << 'EOF'
      echo "$PPID" > activation_pid

      ACTIVATIONS_DIR=$(dirname "$_FLOX_ACTIVATION_STATE_DIR")
      ACTIVATIONS_JSON="${ACTIVATIONS_DIR}/activations.json"
      jq_edit "$ACTIVATIONS_JSON" '.version = 0'

      "$FLOX_BIN" activate -- echo "should fail"
EOF
  )

  # Capture from the previous activation.
  ACTIVATION_PID=$(cat activation_pid)

  assert_failure
  refute_line "should fail"
  assert_output "Error: This environment has already been activated with an incompatible version of 'flox'.

Exit all activations of the environment and try again.
PIDs of the running activations: ${ACTIVATION_PID}"
}

# bats test_tags=activate,activate:attach
@test "version: upgrades the activations.json version" {
  project_setup

  # This has to be updated with [flox_core::activations::LATEST_VERSION].
  LATEST_VERSION=1

  export -f jq_edit
  run "$FLOX_BIN" activate -- bash <(
    cat << 'EOF'
      ACTIVATIONS_DIR=$(dirname "$_FLOX_ACTIVATION_STATE_DIR")
      ACTIVATIONS_JSON="${ACTIVATIONS_DIR}/activations.json"

      jq_edit "$ACTIVATIONS_JSON" '.version = 0'
      echo "$ACTIVATIONS_JSON" > activations_json
EOF
  )
  assert_success

  # Capture from the previous activation.
  ACTIVATIONS_JSON=$(cat activations_json)

  # Wait for the "start" to exit.
  # Add some output to the buffer to debug later assertion failures.
  echo "$(date -u +'%FT%T.%6NZ'): Initial activation finished."
  wait_for_watchdogs "$PROJECT_DIR"
  cat "${PROJECT_DIR}"/.flox/log/watchdog.*

  # Old version should still be recorded.
  jq --exit-status '.version == 0' "$ACTIVATIONS_JSON"

  # New "start" with old version should succeed.
  run "$FLOX_BIN" activate -- echo "should succeed"
  assert_success
  assert_line "should succeed"

  # Version should be upgraded by "start" when there are no other activations.
  jq --exit-status ".version == ${LATEST_VERSION}" "$ACTIVATIONS_JSON"
}

# ---------------------------------------------------------------------------- #

# Sub-commands like `flox-activations` and `flox-watchdog` depend on this.
@test "activate: sets FLOX_DISABLE_METRICS from config" {
  project_setup

  # Set in isolated config.
  "$FLOX_BIN" config --set-bool disable_metrics true
  # Unset from test suite.
  unset FLOX_DISABLE_METRICS

  run --separate-stderr "$FLOX_BIN" activate -- printenv FLOX_DISABLE_METRICS
  assert_output "true"
}

# ---------------------------------------------------------------------------- #

@test "can use fallback interpreter" {
  project_setup
  run "$FLOX_BIN" activate --use-fallback-interpreter -- true
  assert_success
}

@test "fallback flag activates with rendered interpreter" {
  project_setup

  # Attempting to use the interpreter bundled with the CLI will fail because
  # we're overriding the variable for the bundled interpreter store path, so
  # we should only be able to activate if the fallback interpreter is used.
  FLOX_INTERPRETER="/foo" run "$FLOX_BIN" activate --use-fallback-interpreter -- true
  assert_success

  FLOX_INTERPRETER="/foo" run "$FLOX_BIN" activate -- true
  assert_failure
}

@test "can use bundled interpreter to mitigate broken bundled interpreter" {
  project_setup

  # Give the environment a stable name
  "$FLOX_BIN" edit -n bad_interpreter

  # Install something to the environment so the out-link exists
  link_name="$NIX_SYSTEM.bad_interpreter.dev"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" "$FLOX_BIN" install hello
  hello_path="$(realpath ".flox/run/$link_name/bin/hello")"

  # Delete the symlink to the environment
  rm .flox/run/*

  # We need a symlink, so we'll put stuff in here and link it into .flox/run
  mkdir ./fake_env
  mkdir ./fake_env/bin
  echo "exit 1" > ./fake_env/activate # this is our dummy interpreter
  chmod +x ./fake_env/activate
  ln -s "$PWD/fake_env" ".flox/run/$link_name"

  # Attempt activation with the bundled interpreter
  run "$FLOX_BIN" activate -- true
  assert_success

  # Attempt activation with the broken interpreter
  run "$FLOX_BIN" activate --use-fallback-interpreter -- true
  assert_failure
}

# ---------------------------------------------------------------------------- #

@test "can activate in dev mode" {
  project_setup

  run "$FLOX_BIN" activate -m dev -- true
  assert_success
}

@test "can activate in run mode" {
  project_setup

  run "$FLOX_BIN" activate -m run -- true
  assert_success
}

# bats test_tags=activate,activate:attach
@test "attach doesn't break MANPATH" {
  project_setup

  # Ensure that an empty MANPATH is replaced with something with a trailing
  # colon so that the default list is honoured as a fallback.
  MANPATH= run "$FLOX_BIN" activate -- sh -c 'echo $MANPATH'
  assert_success
  assert_output --regexp ".*:$"

  "$FLOX_BIN" init -d vim
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.json" "$FLOX_BIN" install -d vim vim

  "$FLOX_BIN" init -d emacs
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/emacs.json" "$FLOX_BIN" install -d emacs emacs

  mkfifo activate_finished
  # Will get cat'ed in teardown
  TEARDOWN_FIFO="$PROJECT_DIR/teardown_activate"
  mkfifo "$TEARDOWN_FIFO"

  case "$NIX_SYSTEM" in
    *-linux)
      VIM_MAN="$(realpath "$PROJECT_DIR/vim/.flox/run/$NIX_SYSTEM.vim.dev/share/man/man1/vim.1.gz")"
      run man --path vim
      assert_failure
      refute_output "$VIM_MAN"

      EMACS_MAN="$(realpath "$PROJECT_DIR/emacs/.flox/run/$NIX_SYSTEM.emacs.dev/share/man/man1/emacs.1.gz")"
      run man --path emacs
      assert_failure
      refute_output "$EMACS_MAN"

      # vim gets added to MANPATH
      "$FLOX_BIN" activate -d vim -- bash -c "man --path vim > output; echo > activate_finished && echo > \"$TEARDOWN_FIFO\"" &
      cat activate_finished
      run cat output
      assert_success
      assert_output "$VIM_MAN"

      # emacs gets added to MANPATH, and then a nested attach also adds vim
      "$FLOX_BIN" activate -d emacs -- \
        bash -c 'man --path emacs > output_emacs_1 && "$FLOX_BIN" activate -d vim -- bash -c "man --path vim > output_vim && man --path emacs > output_emacs_2"'
      run cat output_emacs_1
      assert_output "$EMACS_MAN"
      run cat output_vim
      assert_output "$VIM_MAN"
      run cat output_emacs_2
      assert_output  "$EMACS_MAN"
      ;;
    *-darwin)
      # Use /usr/bin/manpath to ensure we're checking macOS behavior
      # Neither environment starts out in MANPATH
      run /usr/bin/manpath
      assert_success
      refute_output --regexp ".*$PROJECT_DIR/vim/.flox/run/$NIX_SYSTEM.vim.dev/share/man.*"
      refute_output --regexp ".*$PROJECT_DIR/emacs/.flox/run/$NIX_SYSTEM.emacs.dev/share/man.*"

      # vim gets added to MANPATH
      "$FLOX_BIN" activate -d vim -- bash -c "/usr/bin/manpath > output && echo > activate_finished && echo > \"$TEARDOWN_FIFO\"" &
      cat activate_finished
      run cat output
      assert_success
      assert_output --regexp ".*$PROJECT_DIR/vim/.flox/run/$NIX_SYSTEM.vim.dev/share/man.*"
      refute_output --regexp ".*$PROJECT_DIR/emacs/.flox/run/$NIX_SYSTEM.emacs.dev/share/man.*"

      # emacs gets added to MANPATH, and then a nested attach also adds vim
      "$FLOX_BIN" activate -d emacs -- \
        bash -c '/usr/bin/manpath > output_1 && "$FLOX_BIN" activate -d vim -- bash -c "/usr/bin/manpath > output_2"'
      run cat output_1
      refute_output --regexp ".*$PROJECT_DIR/vim/.flox/run/$NIX_SYSTEM.vim.dev/share/man.*"
      assert_output --regexp ".*$PROJECT_DIR/emacs/.flox/run/$NIX_SYSTEM.emacs.dev/share/man.*"
      run cat output_2
      assert_output --regexp ".*$PROJECT_DIR/vim/.flox/run/$NIX_SYSTEM.vim.dev/share/man.*"
      assert_output --regexp ".*$PROJECT_DIR/emacs/.flox/run/$NIX_SYSTEM.emacs.dev/share/man.*"
      ;;
    *)
      echo "unsupported system: $NIX_SYSTEM"
      return 1
      ;;
  esac
}

# bats test_tags=activate,activate:attach
@test "attach doesn't break PATH" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  "$FLOX_BIN" init -d vim
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim.json" "$FLOX_BIN" install -d vim vim

  "$FLOX_BIN" init -d emacs
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/emacs.json" "$FLOX_BIN" install -d emacs emacs

  mkfifo activate_finished
  # Will get cat'ed in teardown
  TEARDOWN_FIFO="$PROJECT_DIR/teardown_activate"
  mkfifo "$TEARDOWN_FIFO"

  run command -v vim
  refute_output "$(realpath "$PROJECT_DIR")/vim/.flox/run/$NIX_SYSTEM.vim.dev/bin/vim"

  run command -v emacs
  refute_output "$(realpath "$PROJECT_DIR")/emacs/.flox/run/$NIX_SYSTEM.emacs.dev/bin/emacs"

  "$FLOX_BIN" activate -d vim -- bash -c "command -v vim > output; echo > activate_finished && echo > \"$TEARDOWN_FIFO\"" &
  cat activate_finished

  run cat output
  assert_success
  assert_output "$(realpath "$PROJECT_DIR")/vim/.flox/run/$NIX_SYSTEM.vim.dev/bin/vim"

  "$FLOX_BIN" activate -d emacs -- \
    bash -c 'command -v emacs > output_emacs_1; "$FLOX_BIN" activate -d vim -- bash -c "command -v vim > output_vim && command -v emacs > output_emacs_2 || true"'
  run cat output_emacs_1
  assert_success
  assert_output "$(realpath "$PROJECT_DIR")/emacs/.flox/run/$NIX_SYSTEM.emacs.dev/bin/emacs"
  run cat output_vim
  assert_success
  assert_output "$(realpath "$PROJECT_DIR")/vim/.flox/run/$NIX_SYSTEM.vim.dev/bin/vim"
  run cat output_emacs_2
  assert_success
  assert_output "$(realpath "$PROJECT_DIR")/emacs/.flox/run/$NIX_SYSTEM.emacs.dev/bin/emacs"
}

# ---------------------------------------------------------------------------- #

@test "runtime: dev dependencies aren't added to PATH" {
  project_setup
  "$FLOX_BIN" edit -n "runtime_project" # give it a stable name
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/almonds.json" "$FLOX_BIN" install almonds
  # `almonds` brings in Python as a development dependency, and we don't want
  # that in runtime mode
  run "$FLOX_BIN" activate -m run -- bash <(cat <<'EOF'
    [ -e "$FLOX_ENV/bin/almonds" ]
    [ ! -e "$FLOX_ENV/bin/python3" ]
EOF
)
  assert_success
}

@test "runtime: packages still added to PATH" {
  project_setup
  "$FLOX_BIN" edit -n "runtime_project" # give it a stable name
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/almonds.json" "$FLOX_BIN" install almonds
  run "$FLOX_BIN" activate -m run -- which almonds
  assert_output --partial ".flox/run/$NIX_SYSTEM.runtime_project.run/bin/almonds"
}

@test "runtime: remains in runtime mode as bottom layer" {
  # Prepare two environments that we're going to layer
  export bottom_layer_dir="$BATS_TEST_TMPDIR/bottom_layer"
  mkdir "$bottom_layer_dir"
  "$FLOX_BIN" init -d "$bottom_layer_dir"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/almonds.json" "$FLOX_BIN" install -d "$bottom_layer_dir" almonds
  export top_layer_dir="$BATS_TEST_TMPDIR/top_layer"
  mkdir "$top_layer_dir"
  "$FLOX_BIN" init -d "$top_layer_dir"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" "$FLOX_BIN" install -d "$top_layer_dir" hello

  run "$FLOX_BIN" activate -m run -d "$bottom_layer_dir" -- bash <(cat <<'EOF'
    # This is where we *would* find `python3` if it was present
    python_path_bottom="$FLOX_ENV/bin/python3"
    if [ "$(command -v python3)" = "$python_path_bottom" ]; then
      exit 1
    fi

    # Layer another environment on top
    source <("$FLOX_BIN" activate -d "$top_layer_dir")

    # Ensure that we don't find Python from the bottom environment
    if [ "$(command -v python3)" = "$python_path_bottom" ]; then
      exit 1
    fi
EOF
)
  assert_success
}

@test "runtime: remains in runtime mode as top layer" {
  # Prepare two environments that we're going to layer
  export bottom_layer_dir="$BATS_TEST_TMPDIR/bottom_layer"
  mkdir "$bottom_layer_dir"
  "$FLOX_BIN" init -d "$bottom_layer_dir"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" "$FLOX_BIN" install -d "$bottom_layer_dir" hello
  export top_layer_dir="$BATS_TEST_TMPDIR/top_layer"
  mkdir "$top_layer_dir"
  "$FLOX_BIN" init -d "$top_layer_dir"
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/almonds.json" "$FLOX_BIN" install -d "$top_layer_dir" almonds

  run "$FLOX_BIN" activate -d "$bottom_layer_dir" -m run  -- bash <(cat <<'EOF'
    # Layer another environment on top
    source <("$FLOX_BIN" activate -m run -d "$top_layer_dir")

    # Ensure that we don't find Python from the bottom environment
    if [ "$(command -v python3)" = "$FLOX_ENV/bin/python3" ]; then
      exit 1
    fi
EOF
)
  assert_success
}

@test "runtime: doesn't set CPATH" {
  project_setup
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" "$FLOX_BIN" install hello
  export outer_cpath="$CPATH"
  run "$FLOX_BIN" activate -m run -- bash <(cat <<'EOF'
    [ "$CPATH" = "$outer_cpath" ]
EOF
)
  assert_success
}

@test "bash: repeat activation in .bashrc doesn't break aliases" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  "$FLOX_BIN" init -d default
  MANIFEST_CONTENTS_DEFAULT="$(cat << "EOF"
    version = 1

    [profile]
    bash = """
      alias default_alias="echo Hello default!"
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS_DEFAULT" | "$FLOX_BIN" edit -d default -f -

  "$FLOX_BIN" init -d project
  MANIFEST_CONTENTS_PROJECT="$(cat << "EOF"
    version = 1

    [profile]
    bash = """
      alias project_alias="echo Hello project!"
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS_PROJECT" | "$FLOX_BIN" edit -d project -f -

  echo "eval \"\$(\"$FLOX_BIN\" activate -d '$PROJECT_DIR/default')\"" >"$HOME/.bashrc.extra"
  # It would be better use bash -i to source .bashrc,
  # but that causes the tests to background because bash -i tries to open
  # /dev/tty.
  # Instead `eval "$(flox activate -d default)"` manually to simulate sourcing
  # .bashrc
  run bash <(cat <<'EOF'
    set -euo pipefail
    eval "$("$FLOX_BIN" activate -d default)"
    echo "$_FLOX_ACTIVE_ENVIRONMENTS"
    # We can't double check the alias has been loaded because bash isn't
    # interactive and discards it
    FLOX_SHELL="bash" NO_COLOR=1 expect "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR/project" "type project_alias && type default_alias"
EOF
)
  assert_success
  assert_output --partial "project_alias is aliased to \`echo Hello project!'"
  assert_output --partial "default_alias is aliased to \`echo Hello default!'"
}

@test "bash: repeat activation in .bashrc creates correct PATH ordering" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  "$FLOX_BIN" init -d default
  MANIFEST_CONTENTS_DEFAULT="$(cat << "EOF"
    version = 1
EOF
  )"
  echo "$MANIFEST_CONTENTS_DEFAULT" | "$FLOX_BIN" edit -d default -f -

  "$FLOX_BIN" init -d project
  MANIFEST_CONTENTS_PROJECT="$(cat << "EOF"
    version = 1
EOF
  )"
  echo "$MANIFEST_CONTENTS_PROJECT" | "$FLOX_BIN" edit -d project -f -

  echo "eval \"\$(\"$FLOX_BIN\" activate -d '$PROJECT_DIR/default')\"" >"$HOME/.bashrc.extra"
  # It would be better use bash -i to source .bashrc,
  # but that causes the tests to background because bash -i tries to open
  # /dev/tty.
  # Instead `eval "$(flox activate -d default)"` manually to simulate sourcing
  # .bashrc
  run bash <(cat <<'EOF'
    set -euo pipefail
    eval "$("$FLOX_BIN" activate -d default)"
    if ! [[ "$PATH" =~ $PROJECT_DIR/default/.flox/run/.*.default.dev/bin ]]; then # to double check we activated the default environment
      echo "default not in PATH: $PATH"
      exit 1
    fi
    FLOX_SHELL="bash" NO_COLOR=1 expect "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR/project" 'echo "$PATH"'
EOF
)
  assert_success
  assert_output --regexp "project/.flox/run/.*.project.dev/bin.*default/.flox/run/.*.default.dev/bin"
}

@test "tcsh: repeat activation in .tcshrc doesn't break aliases" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  "$FLOX_BIN" init -d default
  MANIFEST_CONTENTS_DEFAULT="$(cat << "EOF"
    version = 1

    [profile]
    tcsh = """
      alias default_alias echo "Hello default!";
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS_DEFAULT" | "$FLOX_BIN" edit -d default -f -

  "$FLOX_BIN" init -d project
  MANIFEST_CONTENTS_PROJECT="$(cat << "EOF"
    version = 1

    [profile]
    tcsh = """
      alias project_alias echo "Hello project!";
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS_PROJECT" | "$FLOX_BIN" edit -d project -f -

  echo "eval \`$FLOX_BIN activate -d '$PROJECT_DIR/default'\`" > "$HOME/.tcshrc.extra"

  # It would be better use tcsh -i to source .tcshrc,
  # but that causes the tests to background because tcsh -i tries to open
  # /dev/tty.
  # Instead `flox activate -d default` manually to simulate sourcing
  # .tcshrc

  export TCSH="$(which tcsh)"
  export EXPECT="$(which expect)"
  run tcsh <(cat <<'EOF'
    set alias_exists="`alias default_alias`"
    if ("$alias_exists" == "") then
      echo "default_alias not found"
      exit 1
    else
    endif
    setenv FLOX_SHELL "$TCSH"
    setenv NO_COLOR 1
    "$EXPECT" "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR/project" "which project_alias && which default_alias"
EOF
)
  assert_success
  assert_output --partial "project_alias: 	 aliased to echo Hello project!"
  assert_output --partial "default_alias: 	 aliased to echo Hello default!"
}

@test "tcsh: repeat activation in .tcshrc creates correct PATH ordering" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  "$FLOX_BIN" init -d default
  MANIFEST_CONTENTS_DEFAULT="$(cat << "EOF"
    version = 1
EOF
  )"
  echo "$MANIFEST_CONTENTS_DEFAULT" | "$FLOX_BIN" edit -d default -f -

  "$FLOX_BIN" init -d project
  MANIFEST_CONTENTS_PROJECT="$(cat << "EOF"
    version = 1
EOF
  )"
  echo "$MANIFEST_CONTENTS_PROJECT" | "$FLOX_BIN" edit -d project -f -

  # It would be better use bash -i to source .bashrc,
  # but that causes the tests to background because bash -i tries to open
  # /dev/tty.
  # Instead `eval "$(flox activate -d default)"` manually to simulate sourcing
  # .bashrc

  echo "eval \`$FLOX_BIN activate -d '$PROJECT_DIR/default'\`" > "$HOME/.tcshrc.extra"


  export TCSH="$(which tcsh)"
  export EXPECT="$(which expect)"
  run tcsh  <(cat <<'EOF'
    setenv FLOX_SHELL "$TCSH"
    setenv NO_COLOR 1
    "$EXPECT" "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR/project" 'echo "$PATH"'
EOF
)
  assert_success
  assert_output --regexp "$PROJECT_DIR/project/.flox/run/.*.project.dev/bin.*$PROJECT_DIR/default/.flox/run/.*.default.dev/bin"
}

@test "fish: repeat activation in config.fish doesn't break aliases" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  "$FLOX_BIN" init -d default
  MANIFEST_CONTENTS_DEFAULT="$(cat << "EOF"
    version = 1

    [profile]
    fish = """
      alias default_alias="echo Hello default!"
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS_DEFAULT" | "$FLOX_BIN" edit -d default -f -

  "$FLOX_BIN" init -d project
  MANIFEST_CONTENTS_PROJECT="$(cat << "EOF"
    version = 1

    [profile]
    fish = """
      alias project_alias="echo Hello project!"
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS_PROJECT" | "$FLOX_BIN" edit -d project -f -

  echo "eval \"\$(\"$FLOX_BIN\" activate -d '$PROJECT_DIR/default')\"" > "$HOME/.config/fish/config.fish.extra"
  # config.fish rewrites PATH from flox-cli-tests
  FISH="$(which fish)"
  EXPECT="$(which expect)"
  run fish <(cat <<EOF
    if ! type default_alias 2&> /dev/null;
      echo "default_alias not found"
      exit 1
    end
    FLOX_SHELL="$FISH" NO_COLOR=1 "$EXPECT" "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR/project" "type project_alias && type default_alias"
EOF
)
  assert_success
  assert_output --partial "project_alias is a function"
  assert_output --partial "default_alias is a function with definition"
}

@test "fish: repeat activation in config.fish creates correct PATH ordering" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  "$FLOX_BIN" init -d default
  MANIFEST_CONTENTS_DEFAULT="$(cat << "EOF"
    version = 1
EOF
  )"
  echo "$MANIFEST_CONTENTS_DEFAULT" | "$FLOX_BIN" edit -d default -f -

  "$FLOX_BIN" init -d project
  MANIFEST_CONTENTS_PROJECT="$(cat << "EOF"
    version = 1
EOF
  )"
  echo "$MANIFEST_CONTENTS_PROJECT" | "$FLOX_BIN" edit -d project -f -

  echo "eval \"\$(\"$FLOX_BIN\" activate -d '$PROJECT_DIR/default')\"" > "$HOME/.config/fish/config.fish.extra"
  # config.fish rewrites PATH from flox-cli-tests
  FISH="$(which fish)"
  EXPECT="$(which expect)"
  run fish <(cat <<EOF
    if not string match -r -- '$PROJECT_DIR/default/.flox/run/.*\.default\.dev/bin' "\$PATH"
      echo "default not in PATH: \$PATH"
      exit 1
    end
    FLOX_SHELL="$FISH" NO_COLOR=1 "$EXPECT" "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR/project" 'echo "\$PATH"'
EOF
)
  assert_success
  assert_output --regexp "$PROJECT_DIR/project/.flox/run/.*.project.dev/bin.*$PROJECT_DIR/default/.flox/run/.*.default.dev/bin"
}

zsh_repeat_activation_aliases() {
  init_files=("$@")

  "$FLOX_BIN" init -d default
  MANIFEST_CONTENTS_DEFAULT="$(cat << "EOF"
    version = 1

    [profile]
    zsh = """
      alias default_alias="echo Hello default!"
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS_DEFAULT" | "$FLOX_BIN" edit -d default -f -

  "$FLOX_BIN" init -d project
  MANIFEST_CONTENTS_PROJECT="$(cat << "EOF"
    version = 1

    [profile]
    zsh = """
      alias project_alias="echo Hello project!"
    """
EOF
  )"
  echo "$MANIFEST_CONTENTS_PROJECT" | "$FLOX_BIN" edit -d project -f -

  for init_file in "${init_files[@]}"; do
    echo "eval \"\$(\"$FLOX_BIN\" activate -d '$PROJECT_DIR/default')\"" >> "$HOME/.$init_file.extra"
  done
  ZSH="$(which zsh)"
  EXPECT="$(which expect)"
  run zsh -i <(cat <<EOF
    set -euo pipefail
    if ! type default_alias > /dev/null; then
      echo "default_alias not found"
      exit 1
    fi
    FLOX_SHELL="$ZSH" NO_COLOR=1 "$EXPECT" "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR/project" "type project_alias && type default_alias"
EOF
)
  assert_success
  assert_output --partial "project_alias is an alias for echo Hello project!"
  assert_output --partial "default_alias is an alias for echo Hello default!"
}

@test "zsh: repeat activation in .zshrc doesn't break aliases" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  zsh_repeat_activation_aliases zshrc
}

@test "zsh: repeat activation in .zshenv doesn't break aliases" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  zsh_repeat_activation_aliases zshenv
}

@test "zsh: repeat activation in .zshenv and .zshrc doesn't break aliases" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  zsh_repeat_activation_aliases zshenv zshrc
}

zsh_repeat_activation_PATH() {
  init_files=("$@")

  "$FLOX_BIN" init -d default
  MANIFEST_CONTENTS_DEFAULT="$(cat << "EOF"
    version = 1
EOF
  )"
  echo "$MANIFEST_CONTENTS_DEFAULT" | "$FLOX_BIN" edit -d default -f -

  "$FLOX_BIN" init -d project
  MANIFEST_CONTENTS_PROJECT="$(cat << "EOF"
    version = 1
EOF
  )"
  echo "$MANIFEST_CONTENTS_PROJECT" | "$FLOX_BIN" edit -d project -f -

  for init_file in "${init_files[@]}"; do
    echo "eval \"\$(\"$FLOX_BIN\" activate -d '$PROJECT_DIR/default')\"" >> "$HOME/$init_file"
  done
  ZSH="$(which zsh)"
  EXPECT="$(which expect)"
  run zsh -i <(cat <<EOF
    set -euo pipefail
    if ! [[ "\$PATH" =~ $PROJECT_DIR/default/.flox/run/.*.default.dev/bin ]]; then # to double check we activated the default environment
      echo "default not in PATH: \$PATH"
      exit 1
    fi
    FLOX_SHELL="$ZSH" NO_COLOR=1 "$EXPECT" "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR/project" 'echo "\$PATH"'
EOF
)
  assert_success
  assert_output --regexp "$PROJECT_DIR/project/.flox/run/.*.project.dev/bin.*$PROJECT_DIR/default/.flox/run/.*.default.dev/bin"
}

@test "zsh: repeat activation in .zshrc creates correct PATH ordering" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  zsh_repeat_activation_PATH .zshrc.extra
}

@test "zsh: repeat activation in .zshenv creates correct PATH ordering" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  # For this test, we don't want .zshrc setting BADPATH since it runs after
  # .zshenv
  rm "$HOME/.zshrc"

  zsh_repeat_activation_PATH .zshenv.extra
}

@test "zsh: repeat activation in .zshenv and .zshrc creates correct PATH ordering" {
  # We don't need an environment, but we do need wait_for_watchdogs to have a
  # PROJECT_DIR to look for
  project_setup_common

  # For this test, we don't want .zshrc setting BADPATH since it runs after
  # .zshenv, and the activation in .zshrc is profile only so it wouldn't fix
  # PATH
  rm "$HOME/.zshrc"

  zsh_repeat_activation_PATH .zshenv.extra .zshrc
}
