#! /usr/bin/env bats
# --------------------------------------------------------------------------- #
#
# @file tests/buildenv.bats
#
# @brief Test building environments from lockfiles.
#
# Relies on lockfiles generated by `pkgdb` from flox manifests.
#
# These tests only check the build segment,
# they do not check the resolution of manifests,
# nor the activation of the resulting environments.
# Such tests are found in `pkgdb` and `flox` respectively.
#
#
# --------------------------------------------------------------------------- #
#
# TODO: Allow a path to a file to be passed.
#
#
# --------------------------------------------------------------------------- #


# bats file_tags=build-env

load setup_suite.bash


# --------------------------------------------------------------------------- #

setup_file() {
  : "${LOCKFILES:=${TESTS_DIR?}/data/buildenv/lockfiles}";
  : "${CAT:=cat}";
  : "${TEST:=test}";
  export LOCKFILES CAT TEST;
}


# ---------------------------------------------------------------------------- #

mk_lock() {
  $PKGDB_BIN manifest lock --ga-registry "$1";
}


# ---------------------------------------------------------------------------- #

# bats test_tags=single,smoke
@test "Simple environment builds successfully" {
  run "$PKGDB_BIN" buildenv "$( < "$LOCKFILES/single-package/manifest.lock"; )";
  assert_success
}

# bats test_tags=single,binaries
@test "Built environment contains binaries" {
  run "$PKGDB_BIN" buildenv                                             \
                   "$( < "$LOCKFILES/single-package/manifest.lock"; )"  \
                   --out-link "$BATS_TEST_TMPDIR/env";
  assert_success;
  assert "$TEST" -x "$BATS_TEST_TMPDIR/env/bin/vim";
}

# bats test_tags=single,activate-files
@test "Built environment contains activate files" {
  run "$PKGDB_BIN" buildenv                                             \
                   "$( < "$LOCKFILES/single-package/manifest.lock"; )"  \
                   --out-link "$BATS_TEST_TMPDIR/env";
  assert_success;
  assert "$TEST" -f "$BATS_TEST_TMPDIR/env/activate/bash";
  assert "$TEST" -f "$BATS_TEST_TMPDIR/env/activate/zsh";
  assert "$TEST" -d "$BATS_TEST_TMPDIR/env/etc/profile.d";
}


# --------------------------------------------------------------------------- #

# bats test_tags=hook,script
@test "Built environment includes hook script" {
  run "$PKGDB_BIN" buildenv "$( < "$LOCKFILES/hook-script/manifest.lock"; )"  \
                            --out-link "$BATS_TEST_TMPDIR/env";
  assert_success;
  assert "$TEST" -f "$BATS_TEST_TMPDIR/env/activate/hook.sh";
  run "$CAT" "$BATS_TEST_TMPDIR/env/activate/hook.sh";
  assert_output "script";
}

# bats test_tags=hook,file
@test "Built environment includes hook file" {
  skip "Hook files require path";
  run "$PKGDB_BIN" buildenv "$( < "$LOCKFILES/hook-file/manifest.lock"; )"  \
                            --out-link "$BATS_TEST_TMPDIR/env";
  assert_success;
  assert "$TEST" -f "$BATS_TEST_TMPDIR/env/activate/hook.sh";
  run "$CAT" "$BATS_TEST_TMPDIR/env/activate/hook.sh";
  assert_output "file";
}


# --------------------------------------------------------------------------- #

# bats test_tags=conflict,detect
@test "Detects conflicting packages" {
  run "$PKGDB_BIN" buildenv "$( < "$LOCKFILES/conflict/manifest.lock"; )"  \
                            --out-link "$BATS_TEST_TMPDIR/env";
  assert_failure;
  assert_output --partial "file conflict between packages";
}

# bats test_tags=conflict,resolve
@test "Allows to resolve conflicting with priority" {
  run "$PKGDB_BIN" buildenv                                                \
                   "$( < "$LOCKFILES/conflict-resolved/manifest.lock"; )"  \
                   --out-link "$BATS_TEST_TMPDIR/env";
  assert_success;
}


# --------------------------------------------------------------------------- #

# bats test_tags=propagated
@test "Environment includes propagated packages" {
    skip "ansi does not work on all systems";
    run "$PKGDB_BIN" buildenv "$( < "$LOCKFILES/propagated/manifest.lock"; )"  \
                              --out-link "$BATS_TEST_TMPDIR/env";
    assert_success;
    # environment contains anki
    # -> which propagates beautifulsoup4
    _PYPKGS="$BATS_TEST_TMPDIR/env/lib/python3.10/site-packages";
    assert "$TEST" -f "$_PYPKGS/bs4/__init__.py";
    # -> which propagates chardet
    assert "$TEST" -f "$_PYPKGS/chardet/__init__.py";
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
