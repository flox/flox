#! /usr/bin/env bash
# ============================================================================ #
#
# Early setup routines used to initialize the test suite.
# This is run once every time `bats' is invoked, but is never rerun between
# individual files or tests.
#
# ---------------------------------------------------------------------------- #

bats_load_library bats-support
bats_load_library bats-assert
bats_require_minimum_version '1.5.0'

# ---------------------------------------------------------------------------- #

# Set the vars `REAL_XDG_{CONFIG,CACHE}_HOME' to point to the user's homedir.
# This allows us to copy some of their existing configs and caches into
# our test harnesses.
# This function does not perform any copies, it only sets variables.
#
# NOTE: we unset these variables past this point to avoid pollution.
xdg_reals_setup() {
  if [[ -n "${__FT_RAN_XDG_REALS_SETUP:-}" ]]; then return 0; fi
  # Set fallbacks and export.
  : "${USER:=$(id -un 2> /dev/null)}"
  if [[ -z "${HOME:-}" ]]; then
    : HOME="$(getent passwd "$USER" 2> /dev/null | cut -d: -f6)"
    if [[ -z "${HOME:-}" ]]; then
      if [[ -d "/home/$USER" ]]; then
        HOME="/home/$USER"
      else
        HOME="${BATS_RUN_TMPDIR:?}/homeless-shelter"
      fi
    fi
  fi
  : "${XDG_CONFIG_HOME:=${HOME:?}/.config}"
  : "${XDG_CACHE_HOME:=$HOME/.cache}"
  : "${XDG_DATA_HOME:=$HOME/.local/share}"
  : "${XDG_STATE_HOME:=$HOME/.local/state}"
  export REAL_USER="$USER"
  export REAL_HOME="$HOME"
  export REAL_XDG_CONFIG_HOME="${XDG_CONFIG_HOME:?}"
  export REAL_XDG_CACHE_HOME="${XDG_CACHE_HOME:?}"
  export REAL_XDG_DATA_HOME="${XDG_DATA_HOME:?}"
  export REAL_XDG_STATE_HOME="${XDG_STATE_HOME:?}"
  # Prevent later routines from referencing real dirs.
  unset USER HOME XDG_CONFIG_HOME XDG_CACHE_HOME XDG_DATA_HOME XDG_STATE_HOME
  export __FT_RAN_XDG_REALS_SETUP=:
}

# ---------------------------------------------------------------------------- #

git_reals_setup() {
  if [[ -n "${__FT_RAN_GIT_REALS_SETUP:-}" ]]; then return 0; fi
  xdg_reals_setup
  # Set fallbacks and export.
  : "${GIT_CONFIG_SYSTEM:=/etc/gitconfig}"
  if [[ -z "${GIT_CONFIG_GLOBAL:-}" ]]; then
    if [[ -r "$REAL_XDG_CONFIG_HOME/git/gitconfig" ]]; then
      GIT_CONFIG_GLOBAL="$REAL_XDG_CONFIG_HOME/git/gitconfig"
    else
      GIT_CONFIG_GLOBAL="${REAL_HOME:?}/.gitconfig"
    fi
  fi
  export REAL_GIT_CONFIG_SYSTEM="${GIT_CONFIG_SYSTEM:?}"
  export REAL_GIT_CONFIG_GLOBAL="${GIT_CONFIG_GLOBAL:?}"
  # Prevent later routines from referencing real configs.
  unset GIT_CONFIG_SYSTEM GIT_CONFIG_GLOBAL
  export __FT_RAN_GIT_REALS_SETUP=:
}

# ---------------------------------------------------------------------------- #

# Prime the flox-gh authentication to use the test credential.
floxtest_gitforge_setup() {
  if [[ -n "${__FT_RAN_FLOXTEST_GITFORGE_SETUP:-}" ]]; then return 0; fi
  xdg_tmp_setup
  flox_vars_setup
  # Create fake flox-gh auth token data recognised as test user on flox
  # gitforge. This obviously won't be recognised as a valid token by the
  # GitHub API, but that's OK because we've hard-coded this identity both
  # in flox-gh and on our gitforge proxy.
  mkdir -p $FLOX_CONFIG_DIR/gh
  cat > $FLOX_CONFIG_DIR/gh/hosts.yml << EOF
github.com:
    oauth_token: flox_testOAuthToken
    user: floxtest
    git_protocol: https
EOF
  chmod 600 $FLOX_CONFIG_DIR/gh/hosts.yml
  export __FT_RAN_FLOXTEST_GITFORGE_SETUP=:
}

# ---------------------------------------------------------------------------- #

print_var() { eval echo "  $1: \$$1"; }

# Backup environment variables pointing to "real" system and users paths.
# We sometimes refer to these in order to copy resources from the system into
# our isolated sandboxes.
reals_setup() {
  nix_store_dir_setup
  xdg_reals_setup
  git_reals_setup
  {
    print_var FLOX_BIN
    print_var NIX_BIN
    print_var NIX_STORE
    print_var REAL_GIT_CONFIG_GLOBAL
    print_var REAL_GIT_CONFIG_SYSTEM
    print_var REAL_HOME
    print_var REAL_USER
    print_var REAL_XDG_CACHE_HOME
    print_var REAL_XDG_CONFIG_HOME
    print_var REAL_XDG_DATA_HOME
    print_var REAL_XDG_STATE_HOME
    print_var TESTS_DIR
  } >&3
}

# ---------------------------------------------------------------------------- #

# Lookup system pair recognized by `nix' for this system.
nix_system_setup() {
  : "${NIX_SYSTEM:=$(
    $NIX_BIN --experimental-features nix-command eval --impure --expr builtins.currentSystem --raw
  )}"
  export NIX_SYSTEM
}

# ---------------------------------------------------------------------------- #

# Lookup the path to the Nix store.
# This is mostly important for testing "single user installs"
nix_store_dir_setup() {
  : "${NIX_STORE:=$(
    $NIX_BIN --experimental-features nix-command eval --impure --expr builtins.storeDir --raw
  )}"
  export NIX_STORE
}

# ---------------------------------------------------------------------------- #

# Set variables related to locating test resources and misc. bats settings.
misc_vars_setup() {
  if [[ -n "${__FT_RAN_MISC_VARS_SETUP:-}" ]]; then return 0; fi

  # Assume that versions:
  # a) start with numbers
  # b) contain at least one dot
  # c) contain only numbers and dots
  #
  # Of course this isn't true in general, but we can adhere to this
  # convention for this set of unit tests.
  #
  # N.B.:
  # - do NOT include $VERSION_REGEX within quotes (eats backslashes)
  # - do NOT add '$' at the end to anchor the match at EOL (doesn't work)
  export VERSION_REGEX='[0-9]+\.[0-9.]+'

  # Used to generate environment names.
  # All envs with this prefix are deleted on startup and exit of this suite.
  export FLOX_TEST_ENVNAME_PREFIX='_testing_'

  # Suppress warnings by `flox create' about environments named with
  # '_testing_*' prefixes.
  export _FLOX_TEST_SUITE_MODE=:

  export __FT_RAN_MISC_VARS_SETUP=:
}

# ---------------------------------------------------------------------------- #

# Scrub vars recognized by `flox' CLI and set a few configurables.
flox_cli_vars_setup() {
  unset FLOX_PROMPT_ENVIRONMENTS _FLOX_ACTIVE_ENVIRONMENTS
  export FLOX_DISABLE_METRICS='true'
}

# ---------------------------------------------------------------------------- #

# Creates an ssh key and sets `SSH_AUTH_SOCK' for use by the test suite.
# It is recommended that you use this setup routine in `setup_suite'.
ssh_key_setup() {
  if [[ -n "${__FT_RAN_SSH_KEY_SETUP:-}" ]]; then return 0; fi
  : "${FLOX_TEST_SSH_KEY:=${BATS_SUITE_TMPDIR?}/ssh/id_ed25519}"
  export FLOX_TEST_SSH_KEY
  if ! [[ -r "$FLOX_TEST_SSH_KEY" ]]; then
    mkdir -p "${FLOX_TEST_SSH_KEY%/*}"
    ssh-keygen -t ed25519 -q -N '' -f "$FLOX_TEST_SSH_KEY" \
      -C 'floxuser@example.invalid'
    chmod 600 "$FLOX_TEST_SSH_KEY"
  fi
  export SSH_AUTH_SOCK="$BATS_SUITE_TMPDIR/ssh/ssh_agent.sock"
  if ! [[ -d "${SSH_AUTH_SOCK%/*}" ]]; then mkdir -p "${SSH_AUTH_SOCK%/*}"; fi
  # If our socket isn't open ( it probably ain't ) we open one.
  if ! [[ -e "$SSH_AUTH_SOCK" ]]; then
    # You can't find work in this town without a good agent. Lets get one.
    eval "$(ssh-agent -s)"
    ln -sf "$SSH_AUTH_SOCK" "$BATS_SUITE_TMPDIR/ssh/ssh_agent.sock"
    export SSH_AUTH_SOCK="$BATS_SUITE_TMPDIR/ssh/ssh_agent.sock"
    ssh-add "$FLOX_TEST_SSH_KEY"
  fi
  unset SSH_ASKPASS
  export __FT_RAN_SSH_KEY_SETUP=:
}

# ---------------------------------------------------------------------------- #

# Create a GPG key to test commit signing.
# The user and email align with `git' and `ssh' identity.
#
# XXX: `gnupg' references `HOME' to lookup keys, which should be set to
#      `$BATS_RUN_TMPDIR/homeless-shelter' by `misc_vars_setup'.
#
# TODO: Secret key signing for `git' blows up this needs to be fixed.
# Tests that require GPG signing are temporarily disabled.
gpg_key_setup() {
  if [[ -n "${__FT_RAN_GPG_KEY_SETUP:-}" ]]; then return 0; fi
  misc_vars_setup
  mkdir -p "$BATS_RUN_TMPDIR/homeless-shelter/.gnupg"
  gpg --full-gen-key --batch <(
    printf '%s\n' \
      'Key-Type: 1' 'Key-Length: 4096' 'Subkey-Type: 1' 'Subkey-Length: 4096' \
      'Expire-Date: 0' 'Name-Real: Flox User' \
      'Name-Email: floxuser@example.invalid' '%no-protection'
  )
  export __FT_RAN_GPG_KEY_SETUP=:
}

# ---------------------------------------------------------------------------- #

# Create a temporary `gitconfig' suitable for this test suite.
gitconfig_setup() {
  if [[ -n "${__FT_RAN_GITCONFIG_SETUP:-}" ]]; then return 0; fi
  git_reals_setup
  mkdir -p "$BATS_SUITE_TMPDIR/git"
  export GIT_CONFIG_SYSTEM="$BATS_SUITE_TMPDIR/git/gitconfig.system"
  # Handle config shared across whole test suite.
  git config --system gpg.format ssh
  # Create a temporary `ssh' key for use by `git'.
  ssh_key_setup
  git config --system user.signingkey "$FLOX_TEST_SSH_KEY.pub"
  # Test files and some individual tests may override this.
  export GIT_CONFIG_GLOBAL="$BATS_SUITE_TMPDIR/git/gitconfig.global"
  touch "$GIT_CONFIG_GLOBAL"
  export __FT_RAN_GITCONFIG_SETUP=:
}

# ---------------------------------------------------------------------------- #

# deleteEnvForce ENV_NAME
# ------------------------
# Force the destruction of an env including any remote metdata.
deleteEnvForce() {
  # TODO delete using Rust
  # { $FLOX_BIN --bash-passthru delete -e "${1?}" --origin -f||:; } >/dev/null 2>&1;
  return 0
}

# ---------------------------------------------------------------------------- #

# Set `XDG_*_HOME' variables to temporary paths.
# This helper should be run after setting `FLOX_TEST_HOME'.
xdg_vars_setup() {
  export XDG_CONFIG_HOME="${FLOX_TEST_HOME:?}/.config"
  export XDG_CACHE_HOME="$FLOX_TEST_HOME/.cache"
  export XDG_DATA_HOME="$FLOX_TEST_HOME/.local/share"
  export XDG_STATE_HOME="$FLOX_TEST_HOME/.local/state"
}

# Copy user's real caches into temporary cache to speed up eval and fetching.
xdg_tmp_setup() {
  xdg_reals_setup
  xdg_vars_setup
  if [[ "${__FT_RAN_XDG_TMP_SETUP:-}" = "${XDG_CACHE_HOME:?}" ]]; then
    return 0
  fi

  # Cache Dirs

  mkdir -p "$XDG_CACHE_HOME"
  chmod u+w "$XDG_CACHE_HOME"
  # We symlink the cache for `nix' so that the fetcher cache and eval cache are
  # shared across the entire suite and between runs.
  # We DO NOT want to use a similar approach for `flox' caches.
  if ! [[ -e "$XDG_CACHE_HOME/nix" ]]; then
    if [[ -e "${REAL_XDG_CACHE_HOME:?}/nix" ]]; then
      chmod u+w "$REAL_XDG_CACHE_HOME/nix"
      ln -s -- "$REAL_XDG_CACHE_HOME/nix" "$XDG_CACHE_HOME/nix"
    else
      mkdir -p "$XDG_CACHE_HOME/nix"
    fi
  fi

  mkdir -p "$XDG_CACHE_HOME/nix/eval-cache-v4"
  chmod u+w "$XDG_CACHE_HOME/nix/eval-cache-v4"

  # Config Dirs

  mkdir -p "${XDG_CONFIG_HOME:?}"
  chmod u+w "$XDG_CONFIG_HOME"
  mkdir -p "$XDG_DATA_HOME/nix"
  chmod u+w "$XDG_DATA_HOME/nix"
  mkdir -p "$XDG_DATA_HOME/flox"
  chmod u+w "$XDG_DATA_HOME/flox"

  # Data Dirs

  mkdir -p "${XDG_DATA_HOME:?}"
  chmod u+w "$XDG_DATA_HOME"
  mkdir -p "$XDG_DATA_HOME/flox"
  chmod u+w "$XDG_DATA_HOME/flox"
  mkdir -p "$XDG_DATA_HOME/flox/environments"
  chmod u+w "$XDG_DATA_HOME/flox/environments"

  # State Dirs

  mkdir -p "${XDG_STATE_HOME:?}"
  chmod u+w "$XDG_STATE_HOME"
  mkdir -p "$XDG_STATE_HOME/flox"
  chmod u+w "$XDG_STATE_HOME/flox"

  export __FT_RAN_XDG_TMP_SETUP="$XDG_CACHE_HOME"
}

# ---------------------------------------------------------------------------- #

# Set variables related to `pkgdb' settings.
pkgdb_vars_setup() {
  if [[ -n "${__FT_RAN_PKGDB_VARS_SETUP:-}" ]]; then return 0; fi

  export _PKGDB_TEST_SUITE_MODE=:

  PKGDB_NIXPKGS_REV_NEW='ab5fd150146dcfe41fda501134e6503932cc8dfd'

  # This causes `pkgdb' to use this revision for `nixpkgs' anywhere the
  # `--ga-registry' flag is used.
  # This is useful for testing `pkgdb' against a specific revision of `nixpkgs'
  # so that we get consistent packages and improved caching.
  _PKGDB_GA_REGISTRY_REF_OR_REV="$PKGDB_NIXPKGS_REV_NEW"

  export PKGDB_NIXPKGS_REV_NEW \
    _PKGDB_GA_REGISTRY_REF_OR_REV

  export __FT_RAN_PKGDB_VARS_SETUP=:
}

# ---------------------------------------------------------------------------- #

# This helper should be run after setting `FLOX_TEST_HOME'.
flox_vars_setup() {
  xdg_vars_setup
  export FLOX_CACHE_HOME="$XDG_CACHE_HOME/flox"
  export FLOX_CONFIG_DIR="$XDG_CONFIG_HOME/flox"
  export FLOX_DATA_HOME="$XDG_DATA_HOME/flox"
  export FLOX_STATE_HOME="$XDG_STATE_HOME/flox"
  export FLOX_META="$FLOX_CACHE_HOME/meta"
  export FLOX_ENVIRONMENTS="$FLOX_DATA_HOME/environments"
  export USER="flox-test"
  export HOME="${FLOX_TEST_HOME:-$HOME}"
}

# ---------------------------------------------------------------------------- #

# home_setup [suite|file|test]
# ----------------------------
# Set `FLOX_TEST_HOME' to a temporary directory and setup essential files.
# Homedirs can be created "globally" for the entire test suite ( default ), or
# for individual files or single tests by passing an optional argument.
home_setup() {
  case "${1:-suite}" in
    suite) export FLOX_TEST_HOME="${BATS_SUITE_TMPDIR?}/home" ;;
    file) export FLOX_TEST_HOME="${BATS_FILE_TMPDIR?}/home" ;;
    test) export FLOX_TEST_HOME="${BATS_TEST_TMPDIR?}/home" ;;
    *)
      echo "home_setup: Invalid homedir category '${1?}'" >&2
      return 1
      ;;
  esac
  #if [[ "${__FT_RAN_HOME_SETUP:-}" = "$FLOX_TEST_HOME" ]]; then return 0; fi
  # Force recreation on `home' on every invocation.
  unset __FT_RAN_HOME_SETUP
  xdg_tmp_setup
  flox_vars_setup
  export __FT_RAN_HOME_SETUP="$FLOX_TEST_HOME"
}

# ---------------------------------------------------------------------------- #

# Shared in common by all members of this test suite.
# Run on startup before all other `*startup' routines.
#
# This function may be extended from external test suites by sourcing this
# script and redefining `setup_suite' with additional routines.
# If you choose to extend this setup please remember that
# `{setup,teardown}_suite' functions must be defined in `setup_suite.bash'
# files, AND keep in mind that `SET_TESTS_DIR' will likely differ.
common_suite_setup() {
  # Backup real env vars.
  reals_setup
  # Setup a phony home directory shared by the test suite.
  # Individual files and tests may create their own private homedirs, but this
  # default one is required before we try to invoke any `flox' CLI commands.
  home_setup suite
  # Set common env vars.
  nix_system_setup
  misc_vars_setup
  flox_cli_vars_setup
  pkgdb_vars_setup
  # Generate configs and auth.
  ssh_key_setup
  floxtest_gitforge_setup
  # TODO: fix gpg setup and re-enable along with `gpgsign.bats' tests.
  #gpg_key_setup;
  gitconfig_setup
  {
    print_var FLOX_TEST_HOME
    print_var HOME
    print_var XDG_CACHE_HOME
    print_var XDG_CONFIG_HOME
    print_var XDG_DATA_HOME
    print_var XDG_STATE_HOME
    print_var FLOX_CACHE_HOME
    print_var FLOX_CONFIG_DIR
    print_var FLOX_DATA_HOME
    print_var FLOX_STATE_HOME
    print_var FLOX_META
    print_var FLOX_ENVIRONMENTS
    print_var NIX_SYSTEM
    print_var FLOX_TEST_SSH_KEY
    print_var SSH_AUTH_SOCK
    print_var GIT_CONFIG_SYSTEM
    print_var GIT_CONFIG_GLOBAL
    print_var PKGDB_NIXPKGS_REV_NEW
    print_var _PKGDB_GA_REGISTRY_REF_OR_REV
  } >&3
}

# Recognized by `bats'.
setup_suite() { common_suite_setup; }

# ---------------------------------------------------------------------------- #

# Shared in common by all members of this test suite.
# Run on exit after all other `*teardown' routines.
common_suite_teardown() {
  # Delete suite tmpdir and envs unless the user requests to preserve them.
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then
    rm -rf "$BATS_SUITE_TMPDIR"
  fi
  # Our agent was useful, but it's time for them to retire.
  # We force true in case we are tearing down when an agent never launched.
  eval "$(ssh-agent -k 2> /dev/null || echo ':')"
  cd "$BAT_RUN_TMPDIR" || return
  # This directory is always deleted because it contains generated secrets.
  # I can't imagine what anyone would ever do with them, but I'm not interested
  # in learning about some esoteric new exploit in an
  # incident response situation because I left them laying around.
  rm -rf "$BATS_RUN_TMPDIR/homeless-shelter"
}

# Recognized by `bats'.
teardown_suite() { common_suite_teardown; }

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
