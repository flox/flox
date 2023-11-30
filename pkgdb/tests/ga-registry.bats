#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# `pkgdb search' tests.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash;

# bats file_tags=search

setup_file() {
  export TDATA="$TESTS_DIR/data/manifest";
  export PROJ1="$TESTS_DIR/harnesses/proj1";

  OTHER_REV="$(
    jq -r '.registry.inputs.nixpkgs.from.rev' "$PROJ1/manifest2.lock";
  )";
  export OTHER_REV;

  # We don't parallelize these to avoid DB sync headaches and to recycle the
  # cache between tests.
  # Nonetheless this file makes an effort to avoid depending on past state in
  # such a way that would make it difficult to eventually parallelize in
  # the future.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true;

  # Change the rev used for the `--ga-registry' flag to align with our cached
  # revision used by other tests.
  # This is both an optimization and a way to ensure consistency of test output.
  export _PKGDB_GA_REGISTRY_REF_OR_REV="$NIXPKGS_REV";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=search:ga-registry

@test "'pkgdb search --help' has '--ga-registry'" {
  run $PKGDB search --help;
  assert_success;
  assert_output --partial "--ga-registry";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=manifest:ga-registry, lock:ga-registry

@test "'pkgdb manifest lock --help' has '--ga-registry'" {
  run $PKGDB manifest lock --help;
  assert_success;
  assert_output --partial "--ga-registry";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=search:ga-registry

# Ensure that the search command succeeds with the `--ga-registry' option and
# no other registry.
@test "'pkgdb search --ga-registry' provides 'global-manifest'" {
  run --separate-stderr sh -c "$PKGDB search --ga-registry --pname hello|wc -l";
  assert_success;
  assert_output 1;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=search:ga-registry

# Ensure that the search command with `--ga-registry' option uses
# `_PKGDB_GA_REGISTRY_REF_OR_REV' as the `nixpkgs' revision.
@test "'pkgdb search --ga-registry' uses '_PKGDB_GA_REGISTRY_REF_OR_REV'" {
  run sh -c "$PKGDB search --ga-registry --pname hello -vv 2>&1 >/dev/null";
  assert_success;
  assert_output --partial "$NIXPKGS_REF";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=search:ga-registry, manifest:ga-registry

@test "'pkgdb search --ga-registry' disallows 'registry' in manifests" {
  run $PKGDB search --ga-registry "{
    \"manifest\": { \"registry\": {} },
    \"query\": { \"pname\": \"hello\" }
  }";
  assert_failure;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=search:ga-registry, manifest:ga-registry

@test "'pkgdb search --ga-registry' disallows 'registry' in global manifests" {
  run $PKGDB search --ga-registry "{
    \"global-manifest\": { \"registry\": {} },
    \"query\": { \"pname\": \"hello\" }
  }";
  assert_failure;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=search:ga-registry, manifest:ga-registry

@test "'pkgdb search --ga-registry' allows 'options' in manifests" {
  run $PKGDB search --ga-registry "{
    \"manifest\": { \"options\": { \"allow\": { \"unfree\": true } } },
    \"query\": { \"pname\": \"hello\" }
  }";
  assert_success;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=search:ga-registry, manifest:ga-registry

@test "'pkgdb search --ga-registry' allows 'options' in global manifests" {
  run $PKGDB search --ga-registry "{
    \"global-manifest\": { \"options\": { \"allow\": { \"unfree\": true } } },
    \"query\": { \"pname\": \"hello\" }
  }";
  assert_success;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=manifest:ga-registry, lock:ga-registry

@test "'pkgdb manifest lock --ga-registry' provides registry" {
  run $PKGDB manifest lock --ga-registry "$TDATA/ga0.toml";
  assert_success;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=manifest:ga-registry, lock:ga-registry, manifest:global

@test "'pkgdb manifest lock --ga-registry' merges global manifest options" {
  run $PKGDB manifest lock --ga-registry                               \
                           --global-manifest "$TDATA/global-ga0.toml"  \
                           "$TDATA/ga0.toml";
  assert_success;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=manifest:ga-registry, lock:ga-registry, manifest:global

@test "'pkgdb manifest lock --ga-registry' rejects global manifest registry" {
  run $PKGDB manifest lock --ga-registry                                     \
                           --global-manifest "$TDATA/global-manifest0.toml"  \
                           "$TDATA/ga0.toml";
  assert_failure;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=manifest:ga-registry, lock:ga-registry

@test "'pkgdb manifest lock --ga-registry' rejects env manifest registry" {
  run $PKGDB manifest lock --ga-registry                               \
                           --global-manifest "$TDATA/global-ga0.toml"  \
                           "$TDATA/post-ga0.toml";
  assert_failure;
}


# ---------------------------------------------------------------------------- #

# bats test_tags=manifest:ga-registry, lock:ga-registry, manifest:registry-cmd

# The lockfile provided contains a `rev` which differs from the `--ga-registry`
# pin used by our test suite.
# This should detect whether the lockfile's `rev` is preserved in `combined`.
@test "Combined registry prefers lockfile inputs" {
  run --separate-stderr                                                \
    sh -c "$PKGDB manifest registry --ga-registry                      \
                                   --lockfile '$PROJ1/manifest2.lock'  \
                                   '$PROJ1/manifest.toml'              \
            |jq -r '.combined.inputs.nixpkgs.from.rev';";
  assert_success;
  assert_output "$OTHER_REV";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=manifest:ga-registry, lock:ga-registry, manifest:update

# The lockfile provided contains a `rev` which differs from the `--ga-registry`
# pin used by our test suite.
# This should cause the `rev` to be updated.
@test "'pkgdb manifest update --ga-registry' updates lockfile rev" {
  run --separate-stderr                                               \
    sh -c "$PKGDB manifest update --ga-registry                       \
                                  --lockfile '$PROJ1/manifest2.lock'  \
                                  '$PROJ1/manifest.toml'              \
            |jq -r '.registry.inputs.nixpkgs.from.rev';";
  assert_success;
  assert_output "$NIXPKGS_REV";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=search:ga-registry, search:lockfile

@test "'pkgdb search --ga-registry' uses lockfile rev" {
  # `$NIXPKGS_REV'
  run --separate-stderr sh -c "$PKGDB search --ga-registry '{
      \"manifest\": \"$PROJ1/manifest.toml\",
      \"lockfile\": \"$PROJ1/manifest.lock\",
      \"query\": { \"match-name\": \"nodejs\" }
    }'|head -n1|jq -r '.version';";
  assert_success;
  assert_output '18.16.0';

  # `$OTHER_REV'
  run --separate-stderr sh -c "$PKGDB search --ga-registry '{
      \"manifest\": \"$PROJ1/manifest.toml\",
      \"lockfile\": \"$PROJ1/manifest2.lock\",
      \"query\": { \"match-name\": \"nodejs\" }
    }'|head -n1|jq -r '.version';";
  assert_success;
  assert_output '18.17.1';
}


# ---------------------------------------------------------------------------- #

# bats test_tags=manifest:ga-registry, lock:ga-registry, manifest:update

@test "'pkgdb manifest update --ga-registry' creates missing lockfile" {
  run --separate-stderr                                             \
    "$PKGDB" manifest update --ga-registry "$PROJ1/manifest.toml";
  assert_success;
  assert_output < "$PROJ1/manifest.lock";
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
