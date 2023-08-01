#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox-bash/lib/manifest.jq' utility.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=manifests, uri, flox-bash


# ---------------------------------------------------------------------------- #

setup() {
  export MANIFESTS_DIR="$TESTS_DIR/manifests";
  # Locate `flox-bash/lib/manifest.jq' file.
  local _prefix;
  _prefix="$( $FLOX_CLI --bash-passthru --prefix; )";
  export MANIFEST_JQ="$_prefix/lib/manifest.jq";
}

manifest() {
  local _manifest="$1"; shift;
  jq -n -e -r -f "$MANIFEST_JQ" --arg system "${NIX_SYSTEM?}"         \
     --slurpfile manifest "$_manifest" --args -- "$@";
}

manifest2() { manifest "$MANIFESTS_DIR/manifest-v2.json" "$@"; }


# ---------------------------------------------------------------------------- #

@test "flakerefToFloxpkg 'github:NixOS/nixpkgs#hello'" {
  run manifest2 flakerefToFloxpkg 'github:NixOS/nixpkgs#hello';
  assert_success;
  assert_output --partial 'github:NixOS/nixpkgs#hello';
}


# ---------------------------------------------------------------------------- #

@test "flakerefToFloxpkg 'github:NixOS/nixpkgs#packages.x86_64-linux.hello'" {
  run manifest2 flakerefToFloxpkg                                    \
                'github:NixOS/nixpkgs#packages.x86_64-linux.hello';
  assert_success;
  assert_output --partial 'github:NixOS/nixpkgs#packages.x86_64-linux.hello';
}


# ---------------------------------------------------------------------------- #

@test "flakerefToFloxpkg 'github:NixOS/nixpkgs#legacyPackages.x86_64-linux.hello'" {
  run manifest2 flakerefToFloxpkg                                          \
                'github:NixOS/nixpkgs#legacyPackages.x86_64-linux.hello';
  assert_success;
  assert_output --partial                                      \
    'github:NixOS/nixpkgs#legacyPackages.x86_64-linux.hello';
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
