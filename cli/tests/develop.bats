#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox develop' subcommand.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=develop

# ---------------------------------------------------------------------------- #

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

# Track the project as a git repository, matching the git-tracking
# prerequisite `flox build`/`flox develop` share for expression builds.
git_init_project() {
  git -C "$PROJECT_DIR" init -q
  git -C "$PROJECT_DIR" config user.name "test"
  git -C "$PROJECT_DIR" config user.email "test@email.address"
}

# Add a git-tracked Nix expression package under `.flox/pkgs/<name>/default.nix`
# and commit it. The expression depends only on plain nixpkgs (`stdenv`,
# `hello`) so no catalog inputs are locked.
nef_package_setup() {
  local name="${1?}"
  local pkg_dir="$PROJECT_DIR/.flox/pkgs/$name"
  mkdir -p "$pkg_dir"
  cat >"$pkg_dir/default.nix" <<'EOF'
{stdenv, hello}:
stdenv.mkDerivation {
  pname = "PNAME_PLACEHOLDER";
  version = "1.0";
  src = ./.;
  buildInputs = [ hello ];
  installPhase = "mkdir -p $out; echo hi > $out/hi";
}
EOF
  sed -i.bak "s/PNAME_PLACEHOLDER/$name/" "$pkg_dir/default.nix"
  rm -f "$pkg_dir/default.nix.bak"
  git -C "$PROJECT_DIR" add -A
  git -C "$PROJECT_DIR" commit -q -m "add $name"
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  home_setup test # Isolate $HOME for each test.
  user_dotfiles_setup
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown() {
  if [ -n "${PROJECT_DIR:-}" ]; then
    project_teardown
  fi
  common_test_teardown
}

# ---------------------------------------------------------------------------- #
# Invocation and refusals

@test "develop: appears in 'flox --help' under 'Use environments'" {
  run "$FLOX_BIN" --help
  assert_success
  assert_output --partial "develop                Enter a development shell for a package build"
}

@test "develop: '--help' documents the package form, not activate's options" {
  run "$FLOX_BIN" develop --help
  assert_success
  assert_output --partial "<package>"
  refute_output --partial "--start-services"
}

@test "develop: refuses a manifest build, naming 'flox activate' and 'sandbox'" {
  project_setup
  MANIFEST_CONTENTS="$(cat <<'EOF'
    version = 1

    [build.greet]
    command = '''
      mkdir -p $out/bin
      echo hi > $out/bin/greet
    '''
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -d "$PROJECT_DIR" -f -

  run "$FLOX_BIN" develop -d "$PROJECT_DIR" greet
  assert_failure
  assert_output --partial "flox activate"
  assert_output --partial "sandbox"
  assert_output --partial "manifest build"
}

@test "develop: refuses an untracked expression file" {
  project_setup
  mkdir -p "$PROJECT_DIR/.flox/pkgs/greet"
  cat >"$PROJECT_DIR/.flox/pkgs/greet/default.nix" <<'EOF'
{stdenv, hello}:
stdenv.mkDerivation {
  pname = "greet";
  version = "1.0";
  src = ./.;
  buildInputs = [ hello ];
  installPhase = "mkdir -p $out; echo hi > $out/hi";
}
EOF
  # No git repository at all: the git-tracking prerequisite refuses before
  # even checking whether the file itself is tracked.
  run "$FLOX_BIN" develop -d "$PROJECT_DIR" greet
  assert_failure
  assert_output --partial "requires git version control"
}

@test "develop: refuses an expression file that is git-tracked-repo but not itself tracked" {
  project_setup
  git_init_project
  mkdir -p "$PROJECT_DIR/.flox/pkgs/greet"
  cat >"$PROJECT_DIR/.flox/pkgs/greet/default.nix" <<'EOF'
{stdenv, hello}:
stdenv.mkDerivation {
  pname = "greet";
  version = "1.0";
  src = ./.;
  buildInputs = [ hello ];
  installPhase = "mkdir -p $out; echo hi > $out/hi";
}
EOF
  run "$FLOX_BIN" develop -d "$PROJECT_DIR" greet
  assert_failure
  assert_output --partial "does not appear to be tracked by git"
}

# ---------------------------------------------------------------------------- #
# Bare invocation: mirrors 'flox build's own convention of resolving the
# project's Nix expression build(s) when none is named.

@test "develop: bare invocation refuses with 'flox build's own no-packages error when the project defines no builds at all" {
  project_setup

  run "$FLOX_BIN" develop -d "$PROJECT_DIR"
  assert_failure
  assert_output --partial "No packages found to build"
}

@test "develop: bare invocation points at 'flox activate' when the project defines only manifest builds" {
  project_setup
  MANIFEST_CONTENTS="$(cat <<'EOF'
    version = 1

    [build.greet]
    command = '''
      mkdir -p $out/bin
      echo hi > $out/bin/greet
    '''
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -d "$PROJECT_DIR" -f -

  run "$FLOX_BIN" develop -d "$PROJECT_DIR"
  assert_failure
  assert_output --partial "flox activate"
  refute_output --partial "No packages found to build"
}

@test "develop: bare invocation refuses with an error naming every candidate, in sorted order, when there is more than one Nix expression build" {
  project_setup
  git_init_project
  nef_package_setup greet
  nef_package_setup farewell

  run "$FLOX_BIN" develop -d "$PROJECT_DIR"
  assert_failure
  # Sorted, joined substring: passes iff the candidates are sorted before
  # being joined, rather than left in `HashMap` iteration order.
  assert_output --partial "farewell, greet"
}

@test "develop: bare invocation enters the shell for the project's sole Nix expression build" {
  project_setup
  git_init_project
  nef_package_setup greet
  export _FLOX_USE_CATALOG_MOCK="$UNIT_TEST_GENERATED/get_base_catalog_nixpkgs_url.yaml"

  run "$FLOX_BIN" develop -d "$PROJECT_DIR" < /dev/null
  assert_success
  assert_output --partial "This shell approximates the build environment for 'greet'"
}

# ---------------------------------------------------------------------------- #
# Divergence disclosure and expression-loop re-entry: these exercise the real
# `eval` -> `nix print-dev-env` pipeline. With no `--stability`/`--nixpkgs-url`
# and no `toplevel` group, resolving a nixpkgs URL calls the base-catalog-info
# endpoint, so these two tests need that endpoint mocked rather than the
# generic empty fixture the rest of this file uses.

@test "develop: discloses the six known divergences on entry" {
  project_setup
  git_init_project
  nef_package_setup greet
  export _FLOX_USE_CATALOG_MOCK="$UNIT_TEST_GENERATED/get_base_catalog_nixpkgs_url.yaml"

  run "$FLOX_BIN" develop -d "$PROJECT_DIR" greet < /dev/null
  assert_success
  assert_output --partial "No build sandbox is applied here"
  assert_output --partial "Your working tree is visible here"
  assert_output --partial "is a snapshot in the Nix store"
  assert_output --partial "point at placeholder paths"
  assert_output --partial "The host PATH stays reachable"
  assert_output --partial "This shell is interactive and sources"
}

@test "develop: enters a shell even when the package's build phases would fail" {
  # `flox develop` realises only the derivation's *inputs* (`nix
  # print-dev-env`), never its builder -- every other fixture in this file
  # builds cleanly, so none of them actually exercises that distinction.
  # This one's `buildPhase` unconditionally fails a real `nix build`; entry
  # must still succeed.
  project_setup
  git_init_project
  local pkg_dir="$PROJECT_DIR/.flox/pkgs/greet"
  mkdir -p "$pkg_dir"
  cat >"$pkg_dir/default.nix" <<'EOF'
{stdenv, hello}:
stdenv.mkDerivation {
  pname = "greet";
  version = "1.0";
  src = ./.;
  buildInputs = [ hello ];
  buildPhase = "exit 1";
  installPhase = "mkdir -p $out; echo hi > $out/hi";
}
EOF
  git -C "$PROJECT_DIR" add -A
  git -C "$PROJECT_DIR" commit -q -m "add greet with a failing buildPhase"
  export _FLOX_USE_CATALOG_MOCK="$UNIT_TEST_GENERATED/get_base_catalog_nixpkgs_url.yaml"

  run "$FLOX_BIN" develop -d "$PROJECT_DIR" greet < /dev/null
  assert_success
  assert_output --partial "This shell approximates the build environment for 'greet'"
}

@test "develop: editing the expression without committing changes the derivation on re-entry" {
  project_setup
  git_init_project
  nef_package_setup greet
  export _FLOX_USE_CATALOG_MOCK="$UNIT_TEST_GENERATED/get_base_catalog_nixpkgs_url.yaml"

  local marker
  marker="$(mktemp)"
  run "$FLOX_BIN" develop -d "$PROJECT_DIR" greet < /dev/null
  assert_success
  first_src="$(grep -m1 '^src=' "$(develop_env_script_path "$marker")")"

  touch "$marker"
  echo "# a comment" >>"$PROJECT_DIR/.flox/pkgs/greet/default.nix"

  run "$FLOX_BIN" develop -d "$PROJECT_DIR" greet < /dev/null
  assert_success
  second_src="$(grep -m1 '^src=' "$(develop_env_script_path "$marker")")"
  rm -f "$marker"

  [ "$first_src" != "$second_src" ]

  # No commit or publish was required for the edit to take effect.
  run git -C "$PROJECT_DIR" status --short
  assert_output --partial ".flox/pkgs/greet/default.nix"
}

@test "develop: shell has stdenv loaded, names the package in the prompt, keeps a host tool reachable, lets a build-input tool of the same name win, and treats \$src as a snapshot" {
  require_expect
  project_setup
  git_init_project
  nef_package_setup greet
  export _FLOX_USE_CATALOG_MOCK="$UNIT_TEST_GENERATED/get_base_catalog_nixpkgs_url.yaml"

  # `user_dotfiles_setup` (called from `setup()`) already points `~/.bashrc`
  # at `BADPATH` and `$KNOWN_PROMPT`, and sources `.bashrc.extra` if present.
  # Add a host-only marker tool and a *fake* `hello` to prove the real,
  # build-input `hello` wins over a same-named host tool.
  mkdir -p "$HOME/fakebin"
  cat >"$HOME/fakebin/my-marker-tool" <<'EOF'
#!/usr/bin/env bash
echo "should never run without exiting nonzero first"
EOF
  chmod +x "$HOME/fakebin/my-marker-tool"
  cat >"$HOME/fakebin/hello" <<'EOF'
#!/usr/bin/env bash
echo "WRONG: this is the host's hello, not the build input"
EOF
  chmod +x "$HOME/fakebin/hello"
  echo "export PATH=\"$HOME/fakebin:\$PATH\"" >"$HOME/.bashrc.extra"

  # `flox develop` always execs the bundled interactive bash regardless of
  # $FLOX_SHELL (the `print-dev-env` output is bash source), so no
  # FLOX_SHELL override is needed here unlike the `activate` exp tests.
  run -0 expect "$TESTS_DIR/develop/develop.exp" "$PROJECT_DIR" greet
  refute_output --partial "WRONG"
}

# Find the `nix print-dev-env` capture written after `$1` (a marker file
# touched immediately before the `flox develop` invocation under test).
# Named apart from the rcfile (which also mentions "src") by locating the
# file that itself starts with the env script's own preamble.
develop_env_script_path() {
  local marker="${1?}"
  find "$FLOX_CACHE_DIR" -type f -newer "$marker" -print0 2>/dev/null \
    | xargs -0 grep -l "^nix_saved_PATH=" 2>/dev/null \
    | head -1
}

# ---------------------------------------------------------------------------- #

# bats test_tags=develop:command
@test "develop: -c runs a command in the dev shell non-interactively and propagates its exit status" {
  project_setup
  git_init_project
  nef_package_setup greet
  export _FLOX_USE_CATALOG_MOCK="$UNIT_TEST_GENERATED/get_base_catalog_nixpkgs_url.yaml"

  # The command sees the dev environment (stdenv's $buildPhase machinery is
  # loaded and $src points into the store), stdout carries only the
  # command's own output, and no disclosure or prompt noise appears.
  run "$FLOX_BIN" develop -d "$PROJECT_DIR" -c 'echo "src=$src"; type genericBuild >/dev/null && echo have-genericBuild'
  assert_success
  assert_line --partial "src=/nix/store/"
  assert_line "have-genericBuild"
  refute_output --partial "This shell approximates the build environment"

  # The command's exit status becomes flox develop's.
  run "$FLOX_BIN" develop -d "$PROJECT_DIR" -c 'exit 7'
  assert_equal "$status" 7
}
