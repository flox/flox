#! /usr/bin/env bash
# ============================================================================ #
#
# Early setup routines used to initialize the test suite.
# This is run once every time `bats' is invoked, but is never rerun between
# individual files or tests.
#
# ---------------------------------------------------------------------------- #

bats_load_library bats-support;
bats_load_library bats-assert;
bats_require_minimum_version '1.5.0';


# ---------------------------------------------------------------------------- #

# Throw errors for undefined variables.
set -u;

# ---------------------------------------------------------------------------- #

# Locate repository root.
repo_root_setup() {
  if [[ -z "${REPO_ROOT:-}" ]]; then
    if [[ -d "$PWD/.git" ]] && [[ -d "$PWD/tests" ]]; then
      REPO_ROOT="$PWD";
    else
      REPO_ROOT="$( git rev-parse --show-toplevel; )";
    fi
  fi
  export REPO_ROOT;
}


# ---------------------------------------------------------------------------- #

# Locate the directory containing test resources.
tests_dir_setup() {
  if [[ -n "${__FT_RAN_TESTS_DIR_SETUP:-}" ]]; then return 0; fi
  repo_root_setup;
  if [[ -z "${TEST_DIR:-}" ]]; then
    case "$BATS_TEST_DIRNAME" in
      */tests) TESTS_DIR="$( readlink -f "$BATS_TEST_DIRNAME"; )"; ;;
      *)       TESTS_DIR="$REPO_ROOT/tests";                       ;;
    esac
    if ! [[ -d "$TESTS_DIR" ]]; then
      echo "tests_dir_setup: \`TESTS_DIR' must be a directory" >&2;
      return 1;
    fi
  fi
  export TESTS_DIR;
  export __FT_RAN_TESTS_DIR_SETUP=:;
}


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
  : "${HOME:-$BATS_RUN_TMPDIR/homeless-shelter}";
  : "${XDG_CONFIG_HOME:-$HOME/.config}";
  : "${XDG_CACHE_HOME:-$HOME/.cache}";
  export REAL_HOME="$HOME";
  export REAL_XDG_CONFIG_HOME="$XDG_CONFIG_HOME";
  export REAL_XDG_CACHE_HOME="$XDG_CACHE_HOME";
  # Prevent later routines from referencing real dirs.
  unset HOME XDG_CONFIG_HOME XDG_CACHE_HOME;
  export __FT_RAN_XDG_REALS_SETUP=:;
}


# ---------------------------------------------------------------------------- #

git_reals_setup() {
  if [[ -n "${__FT_RAN_GIT_REALS_SETUP:-}" ]]; then return 0; fi
  xdg_reals_setup;
  # Set fallbacks and export.
  : "${GH_CONFIG_DIR:=$REAL_XDG_CONFIG_HOME/gh}";
  : "${GIT_CONFIG_SYSTEM:=/etc/gitconfig}";
  if [[ -z "${GIT_CONFIG_GLOBAL:-}" ]]; then
    if [[ -r "$REAL_XDG_CONFIG_HOME/git/gitconfig" ]]; then
      GIT_CONFIG_GLOBAL="$REAL_XDG_CONFIG_HOME/git/gitconfig";
    else
      GIT_CONFIG_GLOBAL="$REAL_HOME/.gitconfig";
    fi
  fi
  export REAL_GH_CONFIG_DIR="$GH_CONFIG_DIR";
  export REAL_GIT_CONFIG_SYSTEM="$GIT_CONFIG_SYSTEM";
  export REAL_GIT_CONFIG_GLOBAL="$GIT_CONFIG_GLOBAL";
  # Prevent later routines from referencing real configs.
  unset GH_CONFIG_DIR GIT_CONFIG_SYSTEM GIT_CONFIG_GLOBAL;
  export __FT_RAN_GIT_REALS_SETUP=:;
}


# ---------------------------------------------------------------------------- #

# Locate the `flox' executable to be tested against.
flox_location_setup() {
  if [[ -n "${__FT_RAN_FLOX_LOCATION_SETUP:-}" ]]; then return 0; fi
  repo_root_setup;
  # Force absolute paths for both FLOX_CLI and FLOX_PACKAGE
  if [[ -z "${FLOX_CLI:-}" ]]; then
    if [[ -x "$REPO_ROOT/target/debug/flox" ]]; then
      FLOX_CLI="$REPO_ROOT/target/debug/flox";
    elif [[ -x "$REPO_ROOT/target/release/flox" ]]; then
      FLOX_CLI="$REPO_ROOT/target/release/flox";
    elif command -v flox &> /dev/null; then
      echo "WARNING: using flox executable from PATH" >&2;
      FLOX_CLI="$( command -v flox; )";
    fi
  fi
  FLOX_CLI="$( readlink -f "$FLOX_CLI"; )";
  export FLOX_CLI;
  export __FT_RAN_FLOX_LOCATION_SETUP=:;
}


# ---------------------------------------------------------------------------- #

# Backup environment variables pointing to "real" system and users paths.
# We sometimes refer to these in order to copy resources from the system into
# our isolated sandboxes.
reals_setup() {
  repo_root_setup;
  tests_dir_setup;
  xdg_reals_setup;
  git_reals_setup;
  flox_location_setup;
}


# ---------------------------------------------------------------------------- #

# Lookup system pair recognized by `nix' for this system.
nix_system_setup() {
  flox_location_setup;
  : "${NIX_SYSTEM:=$(
    $FLOX_CLI nix eval --impure --expr builtins.currentSystem --raw;
  )}";
  export NIX_SYSTEM;
}


# ---------------------------------------------------------------------------- #

# Set variables related to locating test resources and misc. bats settings.
misc_vars_setup() {
  if [[ -n "${__FT_RAN_MISC_VARS_SETUP:-}" ]]; then return 0; fi

  # Set homedir so `parallels' and other tools work, without pollution.
  export HOME="$BATS_RUN_TMPDIR/homeless-shelter";

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
  export VERSION_REGEX='[0-9]+\.[0-9.]+';

  # Used to generate environment names.
  # All envs with this prefix are destroyed on startup and exit of this suite.
  export FLOX_TEST_ENVNAME_PREFIX='_testing_';

  # Suppress warnings by `flox create' about environments named with
  # '_testing_*' prefixes.
  export _FLOX_TEST_SUITE_MODE=:;

  export __FT_RAN_MISC_VARS_SETUP=:;
}


# ---------------------------------------------------------------------------- #

# Scrub vars recognized by `flox' CLI and set a few configurables.
flox_cli_vars_setup() {
  unset FLOX_PROMPT_ENVIRONMENTS FLOX_ACTIVE_ENVIRONMENTS;
  export FLOX_DISABLE_METRICS='true';
}


# ---------------------------------------------------------------------------- #

# Creates an ssh key and sets `SSH_AUTH_SOCK' for use by the test suite.
# It is recommended that you use this setup routine in `setup_suite'.
ssh_key_setup() {
  if [[ -n "${__FT_RAN_SSH_KEY_SETUP:-}" ]]; then return 0; fi
  : "${FLOX_TEST_SSH_KEY:=${BATS_SUITE_TMPDIR?}/ssh/id_ed25519}";
  export FLOX_TEST_SSH_KEY;
  if ! [[ -r "$FLOX_TEST_SSH_KEY" ]]; then
    mkdir -p "${FLOX_TEST_SSH_KEY%/*}";
    ssh-keygen -t ed25519 -q -N '' -f "$FLOX_TEST_SSH_KEY"  \
               -C 'floxuser@example.invalid';
    chmod 600 "$FLOX_TEST_SSH_KEY";
  fi
  export SSH_AUTH_SOCK="$BATS_SUITE_TMPDIR/ssh/ssh_agent.sock";
  if ! [[ -d "${SSH_AUTH_SOCK%/*}" ]]; then mkdir -p "${SSH_AUTH_SOCK%/*}"; fi
  # If our socket isn't open ( it probably ain't ) we open one.
  if ! [[ -e "$SSH_AUTH_SOCK" ]]; then
    # You can't find work in this town without a good agent. Lets get one.
    eval "$( ssh-agent -s; )";
    ln -sf "$SSH_AUTH_SOCK" "$BATS_SUITE_TMPDIR/ssh/ssh_agent.sock";
    export SSH_AUTH_SOCK="$BATS_SUITE_TMPDIR/ssh/ssh_agent.sock";
    ssh-add "$FLOX_TEST_SSH_KEY";
  fi
  unset SSH_ASKPASS;
  export __FT_RAN_SSH_KEY_SETUP=:;
}


# ---------------------------------------------------------------------------- #

# Create a GPG key to test commit signing.
# The user and email align with `git' and `ssh' identity.
#
# XXX: `gnupg' references `HOME' to lookup keys, which should be set to
#      `$BATS_RUN_TMPDIR/homeless-shelter' by `misc_vars_setup'.
gpg_key_setup() {
  if [[ -n "${__FT_RAN_GPG_KEY_SETUP:-}" ]]; then return 0; fi
  misc_vars_setup;
  mkdir -p "$BATS_RUN_TMPDIR/homeless-shelter/.gnupg";
  gpg --full-gen-key --batch <( printf '%s\n'                                \
    'Key-Type: 1' 'Key-Length: 4096' 'Subkey-Type: 1' 'Subkey-Length: 4096'  \
    'Expire-Date: 0' 'Name-Real: Flox User'                                  \
    'Name-Email: floxuser@example.invalid' '%no-protection';
  );
  export __FT_RAN_GPG_KEY_SETUP=:;
}


# ---------------------------------------------------------------------------- #

# Create a temporary `gitconfig' suitable for this test suite.
gitconfig_setup() {
  if [[ -n "${__FT_RAN_GITCONFIG_SETUP:-}" ]]; then return 0; fi
  git_reals_setup;
  mkdir -p "$BATS_SUITE_TMPDIR/git";
  export GIT_CONFIG_SYSTEM="$BATS_SUITE_TMPDIR/git/gitconfig.system";
  # Handle config shared across whole test suite.
  git config --system user.name  'Flox User';
  git config --system user.email 'floxuser@example.invalid';
  git config --system gpg.format ssh;
  # Create a temporary `ssh' key for use by `git'.
  ssh_key_setup;
  git config --system user.signingkey "$FLOX_TEST_SSH_KEY.pub";
  # Test files and some individual tests may override this.
  export GIT_CONFIG_GLOBAL="$BATS_SUITE_TMPDIR/git/gitconfig.global";
  touch "$GIT_CONFIG_GLOBAL";
  export __FT_RAN_GITCONFIG_SETUP=:;
}


# ---------------------------------------------------------------------------- #

# destroyEnvForce ENV_NAME
# ------------------------
# Force the destruction of an env including any remote metdata.
destroyEnvForce() {
  flox_location_setup;
  { $FLOX_CLI destroy -e "${1?}" --origin -f||:; } >/dev/null 2>&1;
}


# Force destroy all test environments.
destroyAllTestEnvs() {
  flox_location_setup;
  misc_vars_setup;
  {
    $FLOX_CLI envs 2>/dev/null                                                 \
      |grep '^[^/[:space:]]\+/'"$FLOX_TEST_ENVNAME_PREFIX"'[[:alnum:]_-]*$'||:;
  }|while read -r e; do destroyEnvForce "$e"; done
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
  reals_setup;
  # Set common env vars.
  nix_system_setup;
  misc_vars_setup;
  flox_cli_vars_setup;
  # Generate configs and auth.
  ssh_key_setup;
  gpg_key_setup;
  gitconfig_setup;
  # Cleanup pollution from past runs.
  destroyAllTestEnvs;
}

# Recognized by `bats'.
setup_suite() { common_suite_setup; }


# ---------------------------------------------------------------------------- #

# Shared in common by all members of this test suite.
# Run on exit after all other `*teardown' routines.
common_suite_teardown() {
  # Delete suite tmpdir and envs unless the user requests to preserve them.
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then
    destroyAllTestEnvs;
    rm -rf "$BATS_SUITE_TMPDIR";
  fi
  # Our agent was useful, but it's time for them to retire.
  eval "$( ssh-agent -k; )";
  # This directory is always deleted because it contains generated secrets.
  # I can't imagine what anyone would ever do with them, but I'm not interested
  # in learning about some esoteric new exploit in an
  # incident response situation because I left them laying around.
  rm -rf "$BATS_RUN_TMPDIR/homeless-shelter";
}

# Recognized by `bats'.
teardown_suite() { common_suite_teardown; }


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
