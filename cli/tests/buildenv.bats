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
  : "${CAT:=cat}"
  : "${TEST:=test}"
  : "${MKDIR:=mkdir}"
  export CAT TEST MKDIR
}

# ---------------------------------------------------------------------------- #

# bats test_tags=single,smoke
@test "Simple environment builds successfully" {
  echo "$BUILDENV_BIN" "$INPUT_DATA/buildenv/lockfiles/single-package/manifest.lock"
  run "$BUILDENV_BIN" "$INPUT_DATA/buildenv/lockfiles/single-package/manifest.lock"
  assert_success
}

# ---------------------------------------------------------------------------- #

# Dropping support for inline JSON because the manifest file is required for
# both `pkgdb realisepkgs` and `buildenv` when building env outputs.

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
  out_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.out')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert "$TEST" -x "${out_outPath}/bin/hello"
  assert "$TEST" -x "${develop_outPath}/bin/hello"
}


# ---------------------------------------------------------------------------- #

# bats test_tags=single,binaries
@test "Built environment contains binaries for v1 flake package" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$INPUT_DATA/buildenv/manual-lockfiles/flake/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  out_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.out')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert "$TEST" -x "${out_outPath}/bin/hello"
  assert "$TEST" -x "${develop_outPath}/bin/hello"
}


# ---------------------------------------------------------------------------- #

# bats test_tags=single,activate-files
@test "Built environment contains activate files" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$INPUT_DATA/buildenv/lockfiles/single-package/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  out_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.out')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert "$TEST" -f "${out_outPath}/activate.d/bash"
  assert "$TEST" -f "${out_outPath}/activate.d/zsh"
  assert "$TEST" -d "${out_outPath}/etc/profile.d"
  assert "$TEST" -f "${develop_outPath}/activate.d/bash"
  assert "$TEST" -f "${develop_outPath}/activate.d/zsh"
  assert "$TEST" -d "${develop_outPath}/etc/profile.d"
}

# --------------------------------------------------------------------------- #

# Dropping support for old v0 `hook.script` manifest attribute.

# ---------------------------------------------------------------------------- #

# bats test_tags=hook,on-activate
@test "Built environment includes 'on-activate' script" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$INPUT_DATA/buildenv/lockfiles/on-activate/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  out_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.out')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert "$TEST" -f "${out_outPath}/activate.d/hook-on-activate"
  assert "$TEST" -f "${develop_outPath}/activate.d/hook-on-activate"
}

# --------------------------------------------------------------------------- #

# bats test_tags=conflict,detect
@test "Detects conflicting packages" {
  run "$BUILDENV_BIN" \
    "$INPUT_DATA/buildenv/lockfiles/conflict/manifest.lock"
  assert_failure
  assert_output --regexp "error: collision between .*-vim-.* and .*-vim-.*"
}

# bats test_tags=conflict,resolve
@test "Allows to resolve onflicting with priority" {
  run --separate-stderr "$BUILDENV_BIN" \
    "$INPUT_DATA/buildenv/lockfiles/conflict-resolved/manifest.lock"
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
    "$INPUT_DATA/buildenv/lockfiles/vars_escape/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  out_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.out')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert "$TEST" -f "${out_outPath}/activate.d/envrc"
  assert "$TEST" -f "${develop_outPath}/activate.d/envrc"
  for i in "${out_outPath}/activate.d/envrc" "${develop_outPath}/activate.d/envrc"; do
    run "$CAT" "$i"
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
  out_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.out')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  build_hello_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs."build-hello"')
  assert "$TEST" -f "${out_outPath}/package-builds.d/hello"
  assert "$TEST" -f "${develop_outPath}/package-builds.d/hello"
  assert "$TEST" -f "${build_hello_outPath}/package-builds.d/hello"
}

# bats test_tags=buildenv:include-lockfile
@test "Built environment contains lockfile" {
  originalLockfile="$GENERATED_DATA/envs/hello/manifest.lock"
  run --separate-stderr "$BUILDENV_BIN" -x \
    "${originalLockfile}"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  out_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.out')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  assert "$TEST" -f "${out_outPath}/manifest.lock"
  assert "$TEST" -f "${develop_outPath}/manifest.lock"
  assert cmp "${out_outPath}/manifest.lock" "${originalLockfile}"
  assert cmp "${develop_outPath}/manifest.lock" "${originalLockfile}"
}


# ---------------------------------------------------------------------------- #

# bats test_tags=requisites
@test "Verify contents of requisites.txt" {
  run --separate-stderr "$BUILDENV_BIN" -x \
    "$INPUT_DATA/buildenv/lockfiles/single-package/manifest.lock"
  assert_success
  assert_equal "${#lines[@]}" 1 # 1 result
  out_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.out')
  develop_outPath=$(echo "${lines[0]}" | jq -er '.[0] | .outputs.develop')
  for i in "${out_outPath}" "${develop_outPath}"; do
    # XXX the contents of requisites.txt is known to be incomplete with
    # this version of buildenv, but the closure represented by the contents
    # of this file should match the closure of the environment itself.
    assert "$TEST" -f "$i/requisites.txt"
    run diff -u "$i/requisites.txt" <(nix-store -qR "$i/." | sort -u)
    assert_success
  done
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
