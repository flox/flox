#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the flox `buildenv' subcommand.
#
# Relies on lockfiles generated from flox manifests.
#
# These tests only check the build segment,
# they do not check the resolution of manifests,
# nor the activation of the resulting environments.
#
#
# --------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=buildenv

# --------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
}

teardown_file() {
  common_file_teardown
}

# ---------------------------------------------------------------------------- #

# bats test_tags=single,smoke
@test "Simple environment builds successfully" {
  run "$BUILDENV_BIN" "$MANUALLY_GENERATED/buildenv/lockfiles/single-package/manifest.lock"
  assert_success
}

# ---------------------------------------------------------------------------- #

# Dropping support for inline JSON because the manifest file is required for
# both `pkgdb realise` and `buildenv` when building env outputs.

# ---------------------------------------------------------------------------- #

# Dropping support for v0 lockfiles because they are no longer supported
# by the nix-based buildenv.

# ---------------------------------------------------------------------------- #

# bats test_tags=single,binaries
@test "Built environment contains binaries for v1 catalog package" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$GENERATED_DATA/envs/hello/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  runtime_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.runtime')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert test -x "${runtime_outPath}/bin/hello"
  assert test -x "${develop_outPath}/bin/hello"
}


# ---------------------------------------------------------------------------- #

# bats test_tags=single,binaries
@test "Built environment contains binaries for v1 flake package" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/manual-lockfiles/flake/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  runtime_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.runtime')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert test -x "${runtime_outPath}/bin/hello"
  assert test -x "${develop_outPath}/bin/hello"
}


# ---------------------------------------------------------------------------- #

# bats test_tags=single,activate-files
@test "Built environment contains activate files" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/lockfiles/single-package/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  runtime_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.runtime')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert test -f "${runtime_outPath}/activate.d/start.bash"
  assert test -f "${runtime_outPath}/activate.d/zsh"
  assert test -d "${runtime_outPath}/etc/profile.d"
  assert test -f "${develop_outPath}/activate.d/start.bash"
  assert test -f "${develop_outPath}/activate.d/zsh"
  assert test -d "${develop_outPath}/etc/profile.d"
}

# --------------------------------------------------------------------------- #

# Dropping support for old v0 `hook.script` manifest attribute.

# ---------------------------------------------------------------------------- #

# bats test_tags=hook,on-activate
@test "Built environment includes 'on-activate' script" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/lockfiles/on-activate/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  runtime_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.runtime')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert test -f "${runtime_outPath}/activate.d/hook-on-activate"
  assert test -f "${develop_outPath}/activate.d/hook-on-activate"
}

# --------------------------------------------------------------------------- #

# bats test_tags=conflict,detect
@test "Detects conflicting packages" {
  run "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/lockfiles/conflict/manifest.lock"
  assert_failure
  assert_output --regexp "error: collision between .*-vim-.* and .*-vim-.*"
}

# bats test_tags=conflict,resolve
@test "Allows to resolve onflicting with priority" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/lockfiles/conflict-resolved/manifest.lock"
  assert_success
}

# ---------------------------------------------------------------------------- #

# Single quotes in variables should be escaped.
# Similarly accidentally escaped single quotes like
#
# [vars]
# singlequoteescaped = "\\'baz"
#
# should be escaped and printed as  \'baz  (literally)
# bats test_tags=buildenv:vars
@test "Environment escapes variables" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/lockfiles/vars_escape/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  runtime_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.runtime')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert test -f "${runtime_outPath}/activate.d/envrc"
  assert test -f "${develop_outPath}/activate.d/envrc"
  for i in "${runtime_outPath}/activate.d/envrc" "${develop_outPath}/activate.d/envrc"; do
    run cat "$i"
    assert_line "export singlequotes=\"'bar'\""
    assert_line "export singlequoteescaped=\"\\'baz\""
  done
}

# bats test_tags=buildenv:build-commands
@test "Built environment contains build script and output" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$GENERATED_DATA/envs/build-noop/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  runtime_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.runtime')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  build_hello_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs."build-hello"')
  assert test -f "${runtime_outPath}/package-builds.d/hello"
  assert test -f "${develop_outPath}/package-builds.d/hello"
  assert test -f "${build_hello_outPath}/package-builds.d/hello"
}

# bats test_tags=buildenv:include-lockfile
@test "Built environment contains lockfile" {
  originalLockfile="$GENERATED_DATA/envs/hello/manifest.lock"
  run --separate-stderr "$BUILDENV_BIN" \
    "${originalLockfile}"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  runtime_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.runtime')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert test -f "${runtime_outPath}/manifest.lock"
  assert test -f "${develop_outPath}/manifest.lock"
  assert cmp "${runtime_outPath}/manifest.lock" "${originalLockfile}"
  assert cmp "${develop_outPath}/manifest.lock" "${originalLockfile}"
}


# ---------------------------------------------------------------------------- #

# bats test_tags=requisites
@test "Verify contents of requisites.txt" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/lockfiles/single-package/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  runtime_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.runtime')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  for i in "${runtime_outPath}" "${develop_outPath}"; do
    # XXX the contents of requisites.txt is known to be incomplete with
    # this version of buildenv, but the closure represented by the contents
    # of this file should match the closure of the environment itself.
    assert test -f "$i/requisites.txt"
    run diff -u "$i/requisites.txt" <(nix-store -qR "$i/." | sort -u)
    assert_success
  done
}

# ---------------------------------------------------------------------------- #

# bats test_tags=buildenv:runtime
@test "Verify build closure contains only toplevel packages" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/lockfiles/build/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  runtime_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.runtime')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  build_myhello_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs."build-myhello"')
  assert test -x "${runtime_outPath}/bin/hello"
  assert test -x "${develop_outPath}/bin/hello"
  assert test -x "${build_myhello_outPath}/bin/hello"
  assert test -x "${runtime_outPath}/bin/coreutils"
  assert test -x "${develop_outPath}/bin/coreutils"
  assert test -x "${build_myhello_outPath}/bin/coreutils"
  assert test -x "${runtime_outPath}/bin/vim"
  assert test -x "${develop_outPath}/bin/vim"
  assert test ! -e "${build_myhello_outPath}/bin/vim"
}

# bats test_tags=buildenv:runtime,buildenv:runtime-packages-only-hello
@test "Verify build closure contains only hello with runtime-packages attribute" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/lockfiles/build-runtime-packages-only-hello/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  runtime_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.runtime')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  build_myhello_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs."build-myhello"')
  assert test -x "${runtime_outPath}/bin/hello"
  assert test -x "${develop_outPath}/bin/hello"
  assert test -x "${build_myhello_outPath}/bin/hello"
  assert test -x "${runtime_outPath}/bin/coreutils"
  assert test -x "${develop_outPath}/bin/coreutils"
  assert test ! -e "${build_myhello_outPath}/bin/coreutils"
  assert test -x "${runtime_outPath}/bin/vim"
  assert test -x "${develop_outPath}/bin/vim"
  assert test ! -e "${build_myhello_outPath}/bin/vim"
}

# bats test_tags=buildenv:runtime,buildenv:runtime-packages-not-toplevel
@test "Verify build closure can only select toplevel packages from runtime-packages attribute" {
  run "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/lockfiles/build-runtime-packages-not-toplevel/manifest.lock"
  assert_failure
  assert_output --regexp "error: package 'vim' is not in 'toplevel' pkg-group"
}

# bats test_tags=buildenv:runtime,buildenv:runtime-packages-not-found
@test "Verify build closure cannot select nonexistent packages in runtime-packages attribute" {
  run "$BUILDENV_BIN" \
    "$MANUALLY_GENERATED/buildenv/lockfiles/build-runtime-packages-not-found/manifest.lock"
  assert_failure
  assert_output --regexp "error: package 'goodbye' not found in '\[install\]' section of manifest"
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
