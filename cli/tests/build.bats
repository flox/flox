#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test `flox build` of Nix expression packages that reference catalog inputs.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=build

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
}

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  "$FLOX_BIN" init -d "$PROJECT_DIR"
  git_init "$PROJECT_DIR"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

# Expression builds require their expressions to be git tracked.
git_init() {
  local dir="${1?}"
  git -C "$dir" init -q
  git -C "$dir" config user.name "test"
  git -C "$dir" config user.email "test@email.address"
  git -C "$dir" config commit.gpgsign false
}

# Add and commit the project's Nix expression package `foo`, whose output
# is the content of the catalog input `catalogs.myorg.hello`.
project_package_setup() {
  local pkg_dir="$PROJECT_DIR/.flox/pkgs/foo"
  mkdir -p "$pkg_dir"
  cat >"$pkg_dir/default.nix" <<'EOF'
{catalogs, runCommand}:
runCommand "foo" {} "cat ${catalogs.myorg.hello} > $out"
EOF
  git -C "$PROJECT_DIR" add -A
  git -C "$PROJECT_DIR" commit -q -m "add foo"
}

# A git repository standing in for the published source of `myorg/hello`:
# a project whose `.flox/pkgs/hello` produces a file containing `$2`.
input_repo_setup() {
  local dir="${1?}"
  local text="${2?}"
  mkdir -p "$dir/.flox/pkgs/hello"
  input_repo_write_hello "$dir" "$text"
  git_init "$dir"
  git -C "$dir" add -A
  git -C "$dir" commit -q -m "add hello"
}

input_repo_write_hello() {
  local dir="${1?}"
  local text="${2?}"
  cat >"$dir/.flox/pkgs/hello/default.nix" <<EOF
{runCommand}:
runCommand "hello" {} "echo -n '$text' > \$out"
EOF
}

# Commit a catalog lock pinning `myorg/hello` to the input repository `$1`
# at its HEAD: the lock `flox build update-catalogs` would write had
# `myorg/hello` been published from that repository, so the build resolves
# the reference with no catalog request.
commit_catalog_lock() {
  local repo="${1?}"
  local rev ref source
  rev="$(git -C "$repo" rev-parse HEAD)"
  ref="$(git -C "$repo" symbolic-ref HEAD)"
  source="{\"dir\": \".flox\", \"ref\": \"$ref\", \"rev\": \"$rev\", \"type\": \"git\", \"url\": \"file://$repo\"}"
  cat >"$PROJECT_DIR/.flox/catalog.lock" <<EOF
{
  "version": 1,
  "direct_catalog_inputs": {
    "myorg/hello": {
      "attr_path": ["hello"],
      "build_type": "nef",
      "catalog": "myorg",
      "inputs": [],
      "locked_inputs_hash": "sha256-test",
      "source": $source
    }
  },
  "catalogs": {
    "myorg": {
      "type": "floxhub",
      "packages": {
        "type": "package_set",
        "entries": {
          "hello": {
            "type": "package",
            "build_type": "nef",
            "source": $source
          }
        }
      }
    }
  }
}
EOF
  git -C "$PROJECT_DIR" add -A
  git -C "$PROJECT_DIR" commit -q -m "lock catalog inputs"
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  home_setup test # Isolate $HOME for each test.
  user_dotfiles_setup
  setup_isolated_flox
  # With no `--stability` and no `toplevel` group, resolving the nixpkgs
  # base URL calls the base-catalog-info endpoint.
  export _FLOX_USE_CATALOG_MOCK="$UNIT_TEST_GENERATED/get_base_catalog_nixpkgs_url.yaml"
  project_setup
  project_package_setup
  export INPUT_REPO="$BATS_TEST_TMPDIR/hello"
  input_repo_setup "$INPUT_REPO" "from the catalog"
  commit_catalog_lock "$INPUT_REPO"
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# bats test_tags=build:override-input
@test "build: '--override-input' fetches the input from the override and leaves the committed lock unchanged" {
  cp "$PROJECT_DIR/.flox/catalog.lock" "$BATS_TEST_TMPDIR/catalog.lock.committed"

  # An uncommitted edit: the locked revision still says "from the catalog".
  input_repo_write_hello "$INPUT_REPO" "from the override"

  run "$FLOX_BIN" build -d "$PROJECT_DIR" foo
  assert_success
  run cat "$PROJECT_DIR/result-foo"
  assert_output "from the catalog"

  run "$FLOX_BIN" build -d "$PROJECT_DIR" --override-input "myorg/hello=$INPUT_REPO" foo
  assert_success
  assert_output --partial "Overriding catalog inputs for this invocation"
  assert_output --partial "'myorg/hello' from 'path:$(cd "$INPUT_REPO" && pwd -P)'"
  run cat "$PROJECT_DIR/result-foo"
  assert_output "from the override"

  run diff "$PROJECT_DIR/.flox/catalog.lock" "$BATS_TEST_TMPDIR/catalog.lock.committed"
  assert_success
}

# bats test_tags=build:override-input
@test "build: '--override-input' naming an input the lock does not pin fails listing the lock's inputs" {
  run "$FLOX_BIN" build -d "$PROJECT_DIR" --override-input "myorg/missing=$INPUT_REPO" foo
  assert_failure
  assert_output --partial "The catalog lock has no input 'myorg/missing'"
  assert_output --partial "Inputs in the lock: myorg/hello"
}

# bats test_tags=build:override-input
@test "build: '--override-input' rejects a value that is not '<KEY>=<FLAKEREF>'" {
  run "$FLOX_BIN" build -d "$PROJECT_DIR" --override-input "myorg/hello" foo
  assert_failure
  assert_output --partial "<KEY>=<FLAKEREF>"
}
