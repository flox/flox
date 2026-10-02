#! /usr/bin/env bats
# -*- mode: bats; -*-

# bats file_tags=build

load test_support.bash

setup() {
  common_test_setup
}

teardown() {
  common_test_teardown
}

@test "NEF resolves an unlisted base attr from the pinned nixpkgs" {
  project="$BATS_TEST_TMPDIR/project"
  mkdir -p "$project/pkgs/base"
  cat >"$project/pkgs/base/default.nix" <<'EOF'
{ catalogs }:
catalogs.nixpkgs.writeText "r03d-base" "from-pinned-nixpkgs"
EOF
  # Direct NEF tests supply the temporary builder-facing JSON, which has .catalogs.
  cat >"$BATS_TEST_TMPDIR/builder-catalog-lock.json" <<'EOF'
{"version":2,"locked_inputs":{},"direct_inputs":[],"catalogs":{}}
EOF

  run nix build --extra-experimental-features 'nix-command flakes' \
    --file "$FLOX_EXPRESSION_BUILD_NIX" \
    --argstr nixpkgs-url "github:NixOS/nixpkgs/$TEST_NIXPKGS_REV_NEW" \
    --argstr source-ref "path:$project" \
    --argstr catalog-lockfile "$BATS_TEST_TMPDIR/builder-catalog-lock.json" \
    --out-link "$BATS_TEST_TMPDIR/result" pkgs.base
  assert_success
  assert [ "$(cat "$BATS_TEST_TMPDIR/result")" = "from-pinned-nixpkgs" ]
}

@test "NEF reserves the base catalog name from lock entries" {
  project="$BATS_TEST_TMPDIR/project"
  mkdir -p "$project/pkgs/base"
  cat >"$project/pkgs/base/default.nix" <<'EOF'
{ catalogs }:
catalogs.nixpkgs.writeText "r03d-base" "from-pinned-nixpkgs"
EOF
  # Direct NEF tests supply the temporary builder-facing JSON, which has .catalogs.
  cat >"$BATS_TEST_TMPDIR/builder-catalog-lock.json" <<'EOF'
{"version":2,"locked_inputs":{},"direct_inputs":[],"catalogs":{"nixpkgs":{"type":"floxhub","packages":{"type":"package_set","entries":{}}}}}
EOF

  run nix eval --extra-experimental-features 'nix-command flakes' \
    --file "$FLOX_EXPRESSION_BUILD_NIX" \
    --argstr nixpkgs-url "github:NixOS/nixpkgs/$TEST_NIXPKGS_REV_NEW" \
    --argstr source-ref "path:$project" \
    --argstr catalog-lockfile "$BATS_TEST_TMPDIR/builder-catalog-lock.json" \
    pkgs.base.drvPath
  assert_failure
  assert_output --partial "reserved nixpkgs catalog"
}
