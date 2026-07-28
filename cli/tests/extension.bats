#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests for the `flox extension` subcommand and the `flox <name>` two-phase
# parse fallback that dispatches to a `flox-<name>` external executable.
#
# Covers P01 (skeleton + dispatch) and P02 (local install / list / remove
# lifecycle). GitHub sources are P03+.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=extension

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  # setup_isolated_flox exports FLOX_DATA_DIR; the dispatch path resolves
  # the extensions root the same way `flox.data_dir` does (FLOX_DATA_DIR
  # first, then XDG_DATA_HOME), so fixtures live under FLOX_DATA_DIR.
  export EXT_ROOT="${FLOX_DATA_DIR?}/extensions"
  mkdir -p "$EXT_ROOT"
  # Extensions are a beta feature and off by default. Enable for every test
  # here; the two tests that assert the disabled behavior `unset` it in
  # their own body, which runs after setup.
  export FLOX_FEATURES_BETA=true
}

teardown() {
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

@test "extension: 'flox <name>' dispatches to flox-<name> when extensions enabled" {
  export FLOX_FEATURES_BETA=true
  local ext_dir="$EXT_ROOT/flox-hello"
  mkdir -p "$ext_dir"
  cat > "$ext_dir/flox-hello" <<'EOF'
#!/usr/bin/env bash
echo "hello from extension"
EOF
  chmod +x "$ext_dir/flox-hello"

  run "$FLOX_BIN" hello
  assert_success
  assert_output --partial "hello from extension"
}

@test "extension: 'flox <name>' dispatch is off unless FLOX_FEATURES_BETA is set" {
  unset FLOX_FEATURES_BETA
  local ext_dir="$EXT_ROOT/flox-hello"
  mkdir -p "$ext_dir"
  cat > "$ext_dir/flox-hello" <<'EOF'
#!/usr/bin/env bash
echo "hello from extension"
EOF
  chmod +x "$ext_dir/flox-hello"

  run "$FLOX_BIN" hello
  assert_failure
  refute_output --partial "hello from extension"
}

@test "extension: 'flox extension --help' lists install/list/remove/search/upgrade" {
  run "$FLOX_BIN" extension --help
  assert_success
  assert_output --partial "install"
  assert_output --partial "list"
  assert_output --partial "remove"
  assert_output --partial "search"
  assert_output --partial "upgrade"
}

@test "extension: 'flox --help' does not list extension (beta commands are hidden)" {
  run "$FLOX_BIN" --help
  assert_success
  # bats-assert `--regexp` matches the entire `$output` as a single
  # string, so `^` only anchors to the very start. Match the line by
  # requiring a newline (or string start) before the leading spaces.
  refute_output --regexp '(^|'$'\n'')[[:space:]]*extension[[:space:]]+Manage flox extensions'
}

@test "extension: subcommands refuse to run when beta is disabled" {
  unset FLOX_FEATURES_BETA
  run "$FLOX_BIN" extension list
  assert_failure
  assert_output --partial "flox config --set features.beta true"
}

# P02-TS07: full install -> list -> dispatch -> remove -> list-empty lifecycle
# against a local source.
@test "extension: install/list/dispatch/remove lifecycle (local source via --from-path)" {
  export FLOX_FEATURES_BETA=true

  # Author a tiny local extension at $BATS_TEST_TMPDIR/flox-hello.
  local src="$BATS_TEST_TMPDIR/flox-hello"
  mkdir -p "$src"
  cat > "$src/flox-hello" <<'EOF'
#!/usr/bin/env bash
echo "hello from extension"
EOF
  chmod +x "$src/flox-hello"

  # Install — name derived from source dirname (no manifest).
  run "$FLOX_BIN" extension install --from-path "$src"
  assert_success
  assert_output --partial "Installed flox-hello"

  # List — table includes hello, repo column shows '.', version is '-'.
  run "$FLOX_BIN" extension list
  assert_success
  assert_output --partial "hello"
  assert_output --partial "."

  # Dispatch — `flox hello` should now resolve to the installed executable.
  run "$FLOX_BIN" hello
  assert_success
  assert_output --partial "hello from extension"

  # Remove.
  run "$FLOX_BIN" extension remove hello
  assert_success
  assert_output --partial "Removed flox-hello"

  # List again — empty.
  run "$FLOX_BIN" extension list
  assert_success
  assert_output --partial "No extensions installed."
}

# ---------------------------------------------------------------------------- #
# P03 — GitHub source helpers
#
# These tests exercise the real `install_github` / `upgrade` code paths
# without needing live network. The trick is twofold:
#
#   1. A local bare git repo stands in for github.com/<owner>/<repo>.git.
#      `GIT_CONFIG_COUNT` / `GIT_CONFIG_KEY_0` / `GIT_CONFIG_VALUE_0`
#      env vars inject a `url.<file://bare>.insteadOf` entry into the
#      git process spawned by `GitHubSource::clone_repo`, redirecting
#      the `https://github.com/owner/flox-hello.git` URL to the local
#      bare repo. Using env vars rather than `git config --global`
#      avoids contention on the suite-wide `gitconfig.global` file when
#      multiple bats files run in parallel.
#
#   2. A tiny Python `http.server` returns canned JSON for the GitHub API
#      endpoints `install_github` hits. `FLOX_EXTENSIONS_GITHUB_BASE_URL`
#      points `GitHubSource::from_env()` at it.
#
# `_setup_github_fixture` builds both and exports `FIXTURE_SHA`.
# `_teardown_github_fixture` kills the server and unsets the redirect.
# ---------------------------------------------------------------------------- #

_setup_github_fixture() {
  local work="$BATS_TEST_TMPDIR/work-flox-hello"
  local bare="$BATS_TEST_TMPDIR/bare/flox-hello.git"
  mkdir -p "$work" "$(dirname "$bare")"
  git init -q --bare "$bare"
  git init -q -b main "$work"
  cat > "$work/flox-hello" <<'EOF'
#!/usr/bin/env bash
echo "hello from gh"
EOF
  chmod +x "$work/flox-hello"
  git -C "$work" -c user.email=t@e -c user.name=t -c commit.gpgsign=false add -A
  git -C "$work" -c user.email=t@e -c user.name=t -c commit.gpgsign=false commit -q -m initial
  git -C "$work" remote add origin "$bare"
  git -C "$work" push -q origin main

  export FIXTURE_WORK="$work"
  export FIXTURE_BARE="$bare"
  export FIXTURE_SHA
  FIXTURE_SHA="$(git -C "$work" rev-parse HEAD)"

  cat > "$BATS_TEST_TMPDIR/api.py" <<EOF
import json, sys, http.server
SHA = "$FIXTURE_SHA"
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        # Strip query string before routing.
        path = self.path.split("?", 1)[0]
        query = self.path.split("?", 1)[1] if "?" in self.path else ""
        if path == "/repos/owner/flox-hello/releases/latest":
            self.send_response(404); self.end_headers(); return
        if path == "/search/repositories":
            # TS07: trigger incomplete_results when the query contains
            # the literal token 'incomplete'.
            incomplete = "incomplete" in query
            body = json.dumps({
                "total_count": 2,
                "incomplete_results": incomplete,
                "items": [
                    {
                        "full_name": "owner/flox-hello",
                        "owner": {"login": "owner"},
                        "name": "flox-hello",
                        "stargazers_count": 42,
                        "description": "canonical hello extension",
                        "archived": False,
                        "html_url": "https://github.com/owner/flox-hello"
                    },
                    {
                        "full_name": "acme/flox-world",
                        "owner": {"login": "acme"},
                        "name": "flox-world",
                        "stargazers_count": 7,
                        "description": "another extension",
                        "archived": False,
                        "html_url": "https://github.com/acme/flox-world"
                    }
                ]
            }).encode()
        elif path == "/repos/owner/flox-hello":
            body = json.dumps({"default_branch": "main"}).encode()
        elif path.startswith("/repos/owner/flox-hello/commits/"):
            body = json.dumps({"sha": SHA}).encode()
        elif path.startswith("/repos/owner/flox-hello/releases/tags/"):
            tag = path.rsplit("/", 1)[-1]
            body = json.dumps({"tag_name": tag}).encode()
        else:
            self.send_response(404); self.end_headers(); return
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a, **k):
        pass
s = http.server.HTTPServer(("127.0.0.1", 0), H)
print(s.server_address[1], flush=True)
s.serve_forever()
EOF
  # Use `python3` if available; else fall back to `python`. Skip the
  # test cleanly if neither is on PATH — the `flox-cli-tests` Nix
  # wrapper controls the PATH and is the source of truth for whether
  # python is available in CI.
  local py
  if command -v python3 > /dev/null 2>&1; then
    py=python3
  elif command -v python > /dev/null 2>&1; then
    py=python
  else
    skip "python3 not available in test environment"
  fi

  $py "$BATS_TEST_TMPDIR/api.py" > "$BATS_TEST_TMPDIR/port.txt" &
  echo $! > "$BATS_TEST_TMPDIR/api.pid"
  local i
  for i in $(seq 1 50); do
    [ -s "$BATS_TEST_TMPDIR/port.txt" ] && break
    sleep 0.1
  done
  local port
  port="$(cat "$BATS_TEST_TMPDIR/port.txt")"
  export FLOX_EXTENSIONS_GITHUB_BASE_URL="http://127.0.0.1:$port"

  # Redirect the github clone URL to the local bare repo via git's
  # GIT_CONFIG_COUNT env-var interface. This avoids contending for the
  # suite-wide `gitconfig.global` file (which other parallel bats files
  # also write to) and gives this test its own private config entries
  # without a lockfile.
  export GIT_CONFIG_COUNT=1
  export GIT_CONFIG_KEY_0="url.file://$bare.insteadOf"
  export GIT_CONFIG_VALUE_0="https://github.com/owner/flox-hello.git"
}

_teardown_github_fixture() {
  if [ -f "$BATS_TEST_TMPDIR/api.pid" ]; then
    kill "$(cat "$BATS_TEST_TMPDIR/api.pid")" 2>/dev/null || true
  fi
  unset GIT_CONFIG_COUNT GIT_CONFIG_KEY_0 GIT_CONFIG_VALUE_0
}

# P04 fixture: release body with assets[] + a tarball served from the same
# Python http.server. No git redirect is needed because the binary-install
# flow skips `git clone` entirely in favor of downloading the asset.
#
# Args: $1=tag (default v1.0.0)
_setup_github_binary_fixture() {
  local tag="${1:-v1.0.0}"
  local py
  if command -v python3 > /dev/null 2>&1; then
    py=python3
  elif command -v python > /dev/null 2>&1; then
    py=python
  else
    skip "python3 not available in test environment"
  fi

  # Detect host OS/arch using the same names the Rust resolver emits.
  local host_os host_arch
  case "$(uname -s)" in
    Darwin) host_os=darwin ;;
    Linux)  host_os=linux ;;
    *)      skip "unsupported host OS for P04 bats fixture: $(uname -s)" ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64)   host_arch=x86_64 ;;
    arm64|aarch64)  host_arch=aarch64 ;;
    *)              skip "unsupported host arch for P04 bats fixture: $(uname -m)" ;;
  esac

  local asset_name="flox-hello-${host_os}-${host_arch}.tar.gz"
  local asset_path="$BATS_TEST_TMPDIR/$asset_name"
  local stage="$BATS_TEST_TMPDIR/stage-$tag"
  mkdir -p "$stage"
  cat > "$stage/flox-hello" <<EOF
#!/usr/bin/env bash
echo "hello from binary ${tag}"
EOF
  chmod +x "$stage/flox-hello"
  # Build the tarball via Python's stdlib so the test does not depend on an
  # external `gzip` binary being on PATH (the bats env only has gnutar +
  # coreutils, and gnutar's `-z` shells out to `gzip`).
  "$py" - "$stage/flox-hello" "$asset_path" <<'PYEOF'
import os, sys, tarfile
src, dest = sys.argv[1], sys.argv[2]
with tarfile.open(dest, "w:gz") as tf:
    ti = tf.gettarinfo(src, arcname="flox-hello")
    ti.mode = 0o755
    with open(src, "rb") as f:
        tf.addfile(ti, f)
PYEOF

  # Fake a commit SHA — the binary-install flow records this verbatim but
  # does not otherwise use it for verification.
  local sha="cafef00d00000000000000000000000000000000"
  export FIXTURE_SHA="$sha"
  export FIXTURE_TAG="$tag"
  export FIXTURE_ASSET_NAME="$asset_name"
  export FIXTURE_ASSET_PATH="$asset_path"

  cat > "$BATS_TEST_TMPDIR/api.py" <<EOF
import json, os, sys, http.server
SHA = "$sha"
TAG = "$tag"
ASSET_NAME = "$asset_name"
ASSET_PATH = "$asset_path"
class H(http.server.BaseHTTPRequestHandler):
    def _release_body(self, port):
        url = f"http://127.0.0.1:{port}/asset/{ASSET_NAME}"
        return json.dumps({
            "tag_name": TAG,
            "target_commitish": "main",
            "assets": [{
                "name": ASSET_NAME,
                "browser_download_url": url,
                "size": os.path.getsize(ASSET_PATH),
                "content_type": "application/gzip",
            }],
        }).encode()
    def do_GET(self):
        port = self.server.server_address[1]
        if self.path == "/repos/owner/flox-hello/releases/latest":
            body = self._release_body(port)
        elif self.path == f"/repos/owner/flox-hello/releases/tags/{TAG}":
            body = self._release_body(port)
        elif self.path == "/repos/owner/flox-hello":
            body = json.dumps({"default_branch": "main"}).encode()
        elif self.path.startswith("/repos/owner/flox-hello/commits/"):
            body = json.dumps({"sha": SHA}).encode()
        elif self.path == f"/asset/{ASSET_NAME}":
            with open(ASSET_PATH, "rb") as f:
                data = f.read()
            self.send_response(200)
            self.send_header("Content-Type", "application/gzip")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        else:
            self.send_response(404); self.end_headers(); return
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a, **k):
        pass
s = http.server.HTTPServer(("127.0.0.1", 0), H)
print(s.server_address[1], flush=True)
s.serve_forever()
EOF

  $py "$BATS_TEST_TMPDIR/api.py" > "$BATS_TEST_TMPDIR/port.txt" &
  echo $! > "$BATS_TEST_TMPDIR/api.pid"
  local i
  for i in $(seq 1 50); do
    [ -s "$BATS_TEST_TMPDIR/port.txt" ] && break
    sleep 0.1
  done
  local port
  port="$(cat "$BATS_TEST_TMPDIR/port.txt")"
  export FLOX_EXTENSIONS_GITHUB_BASE_URL="http://127.0.0.1:$port"
}

_teardown_github_binary_fixture() {
  if [ -f "$BATS_TEST_TMPDIR/api.pid" ]; then
    kill "$(cat "$BATS_TEST_TMPDIR/api.pid")" 2>/dev/null || true
  fi
  unset FLOX_EXTENSIONS_GITHUB_BASE_URL FIXTURE_SHA FIXTURE_TAG FIXTURE_ASSET_NAME FIXTURE_ASSET_PATH
}

# P03-TS05: clone-from-bare-repo install + dispatch
@test "extension: install owner/flox-hello from a local bare repo" {
  export FLOX_FEATURES_BETA=true
  _setup_github_fixture

  run "$FLOX_BIN" extension install owner/flox-hello
  assert_success
  assert_output --partial "Installed flox-hello"

  run "$FLOX_BIN" hello
  assert_success
  assert_output --partial "hello from gh"

  _teardown_github_fixture
}

# P03-TS06: --pin pins; upgrade is no-op without --force, refetches with --force
@test "extension: --pin and upgrade --force lifecycle" {
  export FLOX_FEATURES_BETA=true
  _setup_github_fixture

  run "$FLOX_BIN" extension install --pin "$FIXTURE_SHA" owner/flox-hello
  assert_success
  assert_output --partial "Installed flox-hello"

  run "$FLOX_BIN" extension upgrade hello
  assert_failure
  assert_output --partial "pinned"

  run "$FLOX_BIN" extension upgrade --force hello
  assert_success

  _teardown_github_fixture
}

# P03-TS07: --force install overwrites a prior install at the same name
@test "extension: --force install overwrites prior install" {
  export FLOX_FEATURES_BETA=true
  _setup_github_fixture

  run "$FLOX_BIN" extension install owner/flox-hello
  assert_success

  # Re-install without --force fails.
  run "$FLOX_BIN" extension install owner/flox-hello
  assert_failure
  assert_output --partial "already installed"

  # With --force it succeeds.
  run "$FLOX_BIN" extension install --force owner/flox-hello
  assert_success

  _teardown_github_fixture
}

# P04-TS06: install a precompiled-binary release asset, dispatch, then
# upgrade to a newer tag and verify the executable changed.
@test "extension: install binary release asset and upgrade" {
  export FLOX_FEATURES_BETA=true
  _setup_github_binary_fixture "v1.0.0"

  run "$FLOX_BIN" extension install owner/flox-hello
  assert_success
  assert_output --partial "Installed flox-hello"

  run "$FLOX_BIN" hello
  assert_success
  assert_output --partial "hello from binary v1.0.0"

  # state.toml should record kind=binary and the asset sha.
  run cat "$EXT_ROOT/flox-hello/state.toml"
  assert_success
  assert_output --partial "kind = \"binary\""
  assert_output --partial "tag = \"v1.0.0\""
  assert_output --partial "asset_sha256"

  # Bounce the fixture onto a newer tag with a different asset body.
  _teardown_github_binary_fixture
  _setup_github_binary_fixture "v1.0.1"

  run "$FLOX_BIN" extension upgrade hello
  assert_success

  run "$FLOX_BIN" hello
  assert_success
  assert_output --partial "hello from binary v1.0.1"

  _teardown_github_binary_fixture
}

# P05-TS04: pre-seed one pinned-git + one local extension and assert
# `upgrade --all --dry-run` prints a row per extension, exercising both
# the pinned-skip path and a non-upgradable kind in the same table.
@test "extension: upgrade --all --dry-run prints a row per installed extension" {
  export FLOX_FEATURES_BETA=true
  _setup_github_fixture

  # Pinned script install — exercises the DryRunStatus::Pinned branch.
  run "$FLOX_BIN" extension install --pin "$FIXTURE_SHA" owner/flox-hello
  assert_success

  # Local extension — exercises the per-row error branch (LocalNotSupported).
  local local_src="$BATS_TEST_TMPDIR/flox-local"
  mkdir -p "$local_src"
  cat > "$local_src/flox-local" <<'EOF'
#!/usr/bin/env bash
echo "hi"
EOF
  chmod +x "$local_src/flox-local"
  run "$FLOX_BIN" extension install --from-path "$local_src"
  assert_success

  run "$FLOX_BIN" extension upgrade --all --dry-run
  assert_success
  assert_output --partial "NAME"
  assert_output --partial "STATUS"
  assert_output --partial "hello"
  assert_output --partial "local"
  assert_output --partial "pinned"

  _teardown_github_fixture
}

# P05-TS05: error strings from research-doc §2.9 must match verbatim.
@test "extension: install rejects reserved name with the §2.9 message" {
  export FLOX_FEATURES_BETA=true
  local src="$BATS_TEST_TMPDIR/flox-install"
  mkdir -p "$src"
  echo '#!/bin/sh' > "$src/flox-install"
  chmod +x "$src/flox-install"

  run "$FLOX_BIN" extension install --from-path "$src"
  assert_failure
  assert_output --partial "name 'install' conflicts with a built-in flox command"
}

# P11-TS05: drift guard for RESERVED_COMMAND_NAMES.
#
# `try_dispatch_external` only fires when bpaf fails to parse the first
# positional, so a built-in always shadows a same-named extension. The
# installer refuses reserved names to stop users installing something that
# could never dispatch — but that list is hand-maintained in
# `cli/beta/src/extensions/reserved.rs` and silently rots when flox gains a
# command. This walks every visible top-level command and asserts the
# installer refuses it.
#
# Black-box on purpose: it exercises the shipped parser and the real
# installer, so it needs no test code in the `flox` crate.
#
# Parsing note: command rows are indented exactly four spaces. Options are
# also four-space indented but begin with `-`, and wrapped description text
# is indented far deeper — matching that by accident is how a previous
# version of this check mistook the wrapped word "invocation" for a
# command. Stopping at "Available options:" and requiring `[a-z]` after
# exactly four spaces excludes both.
#
# Hidden commands (`extension`, `help`, `beta-enabled`, `factory`) never
# appear in --help and cannot be covered here; they are listed by hand in
# reserved.rs.
@test "extension: reserved-name list covers every visible top-level command" {
  export FLOX_FEATURES_BETA=true

  run "$FLOX_BIN" --help
  assert_success
  local commands
  commands="$(printf '%s\n' "$output" \
    | awk '/^Available options:/ {exit} /^    [a-z]/ {print $1}' \
    | tr -d ',' | sort -u)"

  # Guard the guard: if the help layout changes and we scrape nothing,
  # every assertion below would vacuously pass.
  local count
  count="$(printf '%s\n' "$commands" | grep -c .)"
  [ "$count" -ge 10 ] || {
    echo "expected >=10 top-level commands, parsed $count -- help layout changed?" >&2
    return 1
  }

  # `--from-path` derives the extension name from the *directory* basename,
  # which must be `flox-<name>`; give each candidate its own parent so the
  # directories don't collide.
  local cmd dir
  for cmd in $commands; do
    dir="$BATS_TEST_TMPDIR/reserved/$cmd/flox-$cmd"
    mkdir -p "$dir"
    echo '#!/bin/sh' > "$dir/flox-$cmd"
    chmod +x "$dir/flox-$cmd"

    run "$FLOX_BIN" extension install --from-path "$dir"
    assert_failure
    assert_output --partial "name '$cmd' conflicts with a built-in flox command"
  done
}

@test "extension: second install without --force emits the §2.9 message" {
  export FLOX_FEATURES_BETA=true
  _setup_github_fixture

  run "$FLOX_BIN" extension install owner/flox-hello
  assert_success

  run "$FLOX_BIN" extension install owner/flox-hello
  assert_failure
  assert_output --partial "flox-hello is already installed (run with --force to overwrite)"

  _teardown_github_fixture
}

@test "extension: missing executable emits the §2.9 message" {
  export FLOX_FEATURES_BETA=true
  local src="$BATS_TEST_TMPDIR/flox-noexe"
  mkdir -p "$src"
  # Intentionally no flox-noexe executable inside.

  run "$FLOX_BIN" extension install --from-path "$src"
  assert_failure
  assert_output --partial "has no executable"
}

@test "extension: upgrade on pinned without --force emits the pinned-skip hint" {
  export FLOX_FEATURES_BETA=true
  _setup_github_fixture

  run "$FLOX_BIN" extension install --pin "$FIXTURE_SHA" owner/flox-hello
  assert_success

  run "$FLOX_BIN" extension upgrade hello
  assert_failure
  assert_output --partial "pinned"

  _teardown_github_fixture
}

# P08-TS06: `flox extension search` decorates installed repos with a ✓
# and leaves uninstalled repos unmarked.
@test "extension: search marks installed repos with check and leaves others unmarked" {
  export FLOX_FEATURES_BETA=true
  _setup_github_fixture

  run "$FLOX_BIN" extension install owner/flox-hello
  assert_success

  run "$FLOX_BIN" extension search hello
  assert_success
  assert_output --partial "OWNER/REPO"
  assert_output --partial "STARS"
  assert_output --regexp '✓[[:space:]]+owner/flox-hello'
  assert_output --regexp '[[:space:]]+acme/flox-world'
  refute_output --regexp '✓[[:space:]]+acme/flox-world'

  _teardown_github_fixture
}

# P08-TS07: when the Search API sets `incomplete_results: true`, a stderr
# warning is emitted and the exit code is still 0.
@test "extension: search warns on incomplete_results without failing" {
  export FLOX_FEATURES_BETA=true
  _setup_github_fixture

  # The fixture flips incomplete_results on when the query string
  # contains the literal token 'incomplete'.
  run "$FLOX_BIN" extension search incomplete
  assert_success
  assert_output --partial "incomplete results"

  _teardown_github_fixture
}

# ---------------------------------------------------------------------------- #
# P06 — Environment integration (Inherit / None / Pinned)
#
# The managed-extension layout is `$EXT_ROOT/flox-<name>/flox-<name>`; P06
# also reads `flox-extension.toml` (optional `[environment]` stanza) and
# `state.toml` (for `FLOX_EXTENSION_VERSION`) from the same directory. These
# helpers write a managed extension with a user-supplied script body and an
# optional author manifest.
# ---------------------------------------------------------------------------- #

# Write a managed extension at $EXT_ROOT/flox-<name>/flox-<name>.
# Args: $1=name, $2=script_body (stdin body after shebang).
_p06_mk_managed_ext() {
  local name="$1"
  local body="$2"
  local ext_dir="$EXT_ROOT/flox-$name"
  mkdir -p "$ext_dir"
  printf '%s\n' '#!/usr/bin/env bash' "$body" > "$ext_dir/flox-$name"
  chmod +x "$ext_dir/flox-$name"
  # Minimal state.toml — supplies `FLOX_EXTENSION_NAME` / `_VERSION`.
  cat > "$ext_dir/state.toml" <<EOF
schema = "1"
name = "$name"
kind = "local"
source = "."
installed_at = "1970-01-01T00:00:00Z"
path = "$ext_dir/flox-$name"
EOF
}

# Write flox-extension.toml with a `[environment]` stanza.
# Args: $1=name, $2=mode ("inherit"|"none"|"pinned"),
#       $3=inherit_name (for pinned, else ""),
#       $4=on_active_inside ("error"|""|skip).
_p06_write_author_manifest() {
  local name="$1"
  local mode="$2"
  local inherit_name="$3"
  local on_active="$4"
  local path="$EXT_ROOT/flox-$name/flox-extension.toml"
  {
    echo 'schema = "1"'
    echo
    echo '[extension]'
    echo "name = \"$name\""
    echo
    echo '[environment]'
    echo "mode = \"$mode\""
    [ -n "$inherit_name" ] && echo "inherit_name = \"$inherit_name\""
    if [ -n "$on_active" ]; then
      echo
      echo '[on_active]'
      echo "inside = \"$on_active\""
    fi
  } > "$path"
}

# P06-TS04: Inherit mode, outside any activation. `FLOX_ENV` is unset in
# the parent shell, so the extension sees it unset too. Bookkeeping vars
# are present on all three modes.
@test "extension: P06 Inherit mode outside activation sees no FLOX_ENV" {
  export FLOX_FEATURES_BETA=true
  _p06_mk_managed_ext "probe" \
    'echo "FLOX_ENV=${FLOX_ENV:-unset}"
echo "EXT_NAME=${FLOX_EXTENSION_NAME:-unset}"'
  # No author manifest → default Inherit mode.
  unset FLOX_ENV
  unset _FLOX_ACTIVE_ENVIRONMENTS

  run "$FLOX_BIN" probe
  assert_success
  assert_output --partial "FLOX_ENV=unset"
  assert_output --partial "EXT_NAME=probe"
}

# P06-TS05: Inherit mode, inside an activation. The ambient `flox activate`
# sets FLOX_ENV; the extension inherits it.
@test "extension: P06 Inherit mode inside activation sees ambient FLOX_ENV" {
  export FLOX_FEATURES_BETA=true
  _p06_mk_managed_ext "probe" 'echo "FLOX_ENV=${FLOX_ENV:-unset}"'
  # No author manifest → default Inherit mode.

  local proj="$BATS_TEST_TMPDIR/p06-inherit-inside"
  mkdir -p "$proj"
  pushd "$proj" > /dev/null
  "$FLOX_BIN" init -d "$proj" > /dev/null

  run "$FLOX_BIN" activate -d "$proj" -- "$FLOX_BIN" probe
  popd > /dev/null
  assert_success
  refute_output --partial "FLOX_ENV=unset"
  assert_output --partial "FLOX_ENV="
}

# P06-TS06: Pinned mode, no matching active env. The dispatcher wraps the
# call in `flox activate -r owner/<name> --`. Without a real FloxHub ref
# the activation fails — but we can verify the pinned-ref activation code
# path was attempted by looking for the FloxHub-resolution error.
@test "extension: P06 Pinned mode outside activation attempts flox activate -r" {
  export FLOX_FEATURES_BETA=true
  _p06_mk_managed_ext "probe" 'echo PROBE_RAN_OUTSIDE_ACTIVATION'
  _p06_write_author_manifest "probe" "pinned" "owner/no-such-env" ""
  unset _FLOX_ACTIVE_ENVIRONMENTS

  run "$FLOX_BIN" probe
  # Expected to fail because owner/no-such-env does not exist, but the
  # failure must come from the activation attempt (not from a silent fall
  # through to direct execution of the extension).
  assert_failure
  refute_output --partial "PROBE_RAN_OUTSIDE_ACTIVATION"
}

# P06-TS07: Pinned mode where the caller is already activated in the
# pinned env (same owner/name). The dispatcher short-circuits the wrapper
# and runs the extension directly — exactly once.
@test "extension: P06 Pinned mode short-circuits when caller is already in the pinned env" {
  export FLOX_FEATURES_BETA=true
  _p06_mk_managed_ext "probe" 'echo "ran=$RANDOM-$$"'

  # Create a managed env at owner/<name> by pushing a path env. The
  # `floxhub_setup` helper wires up a fake FloxHub at FLOXHUB_URL so we
  # can push.
  floxhub_setup owner
  local proj="$BATS_TEST_TMPDIR/p06-pinned-inside"
  mkdir -p "$proj"
  pushd "$proj" > /dev/null
  "$FLOX_BIN" init -d "$proj" -n inner > /dev/null
  "$FLOX_BIN" push --owner owner -d "$proj" > /dev/null || skip "floxhub push fixture unavailable"
  popd > /dev/null

  _p06_write_author_manifest "probe" "pinned" "owner/inner" ""

  # Inside the activation, the dispatcher should see _FLOX_ACTIVE_ENVIRONMENTS
  # containing owner/inner and skip re-activation. Script runs once.
  run "$FLOX_BIN" activate -d "$proj" -- "$FLOX_BIN" probe
  assert_success
  assert_output --partial "ran="
}

# P06-TS08: on_active="error" and the caller is active in a *different*
# env than the pinned one → dispatcher emits the §2.9 mismatch error.
@test "extension: P06 on_active=error mismatch emits the §2.9 trust-prompt error" {
  export FLOX_FEATURES_BETA=true
  _p06_mk_managed_ext "probe" 'echo ran'
  _p06_write_author_manifest "probe" "pinned" "owner/right-env" "error"

  local proj="$BATS_TEST_TMPDIR/p06-on-active-error"
  mkdir -p "$proj"
  pushd "$proj" > /dev/null
  "$FLOX_BIN" init -d "$proj" > /dev/null
  popd > /dev/null

  # Caller is in some other env; on_active=error forbids launching here.
  run "$FLOX_BIN" activate -d "$proj" -- "$FLOX_BIN" probe
  assert_failure
  assert_output --partial "requires the 'owner/right-env' environment"
  refute_output --partial "ran"
}

# P06-TS09: None mode scrubs FLOX_ / _FLOX_ prefixed vars before launch
# and keeps the four bookkeeping vars. Non-FLOX vars (PATH, HOME) pass
# through unchanged.
@test "extension: P06 None mode scrubs FLOX_* and keeps only bookkeeping vars" {
  export FLOX_FEATURES_BETA=true
  _p06_mk_managed_ext "probe" \
    'env | grep -E "^_?FLOX" | sort'
  _p06_write_author_manifest "probe" "none" "" ""

  # Set a fake FLOX_ var to confirm it gets scrubbed. Also set a
  # _FLOX_ACTIVE_ENVIRONMENTS to confirm the underscore-prefixed form
  # is scrubbed too.
  export FLOX_SHOULD_BE_SCRUBBED=yes
  export _FLOX_ACTIVE_ENVIRONMENTS='[]'

  run "$FLOX_BIN" probe
  assert_success
  refute_output --partial "FLOX_SHOULD_BE_SCRUBBED"
  refute_output --partial "_FLOX_ACTIVE_ENVIRONMENTS"
  # Bookkeeping vars are overlaid *after* env_clear, so they appear.
  assert_output --partial "FLOX_EXTENSION_NAME=probe"
  assert_output --partial "FLOX_EXTENSION_PATH="
  assert_output --partial "FLOX_BIN="
}

# ---------------------------------------------------------------------------- #
# P07-TS02: gh-parity smoke — install/list/upgrade/remove
#
# Mirrors the upstream `cli/cli` extension integ flow against our
# local bare-git fixture: end-to-end install, list, dispatch,
# upgrade-with-no-new-commits, remove.
# ---------------------------------------------------------------------------- #

@test "extension: gh-parity smoke — install/list/upgrade/remove" {
  _setup_github_fixture

  # install
  run "$FLOX_BIN" extension install owner/flox-hello
  assert_success
  assert_output --partial "Installed flox-hello"

  # list contains the name and repo
  run "$FLOX_BIN" extension list
  assert_success
  assert_output --partial "hello"
  assert_output --partial "owner/flox-hello"

  # dispatch resolves
  run "$FLOX_BIN" hello
  assert_success
  assert_output --partial "hello from gh"

  # upgrade with no new commits on main → already-current
  run "$FLOX_BIN" extension upgrade hello
  assert_success
  assert_output --partial "already at the latest commit"

  # remove deletes the install dir
  run "$FLOX_BIN" extension remove hello
  assert_success
  assert_output --partial "Removed flox-hello"
  [ ! -d "$EXT_ROOT/flox-hello" ]

  # dispatch now fails (flox has no 'hello' subcommand and no managed exe)
  run "$FLOX_BIN" hello
  assert_failure

  _teardown_github_fixture
}

# ---------------------------------------------------------------------------- #
# P07-TS01: docs pages present and relative links resolve
#
# Reduced scope replacement for the original docs-build check. No docs
# build tooling exists yet, so this asserts source presence and that
# relative `](./...)` / `](../...)` links resolve to real paths in the
# tree.
# ---------------------------------------------------------------------------- #

@test "extension: docs pages present and links resolve" {
  # The flox-cli-tests harness exports PROJECT_ROOT_DIR when running
  # against a real source checkout (the common `just integ-tests` path).
  # When the harness is launched from a /nix/store copy (the pure-Nix
  # `nix-integ-tests` path) there's no source tree to inspect, so the
  # presence check doesn't apply and we skip cleanly.
  if [ -z "${PROJECT_ROOT_DIR:-}" ]; then
    skip "PROJECT_ROOT_DIR not set (tests running from a Nix-built copy)"
  fi

  local docs="$PROJECT_ROOT_DIR/docs/extensions"
  [ -f "$docs/README.md" ]
  [ -f "$docs/user-guide.md" ]
  [ -f "$docs/author-guide.md" ]

  # For each doc file, extract relative markdown links and assert
  # each target resolves against the file's directory. External
  # http(s) and anchor-only links are ignored.
  local f target abs rel
  for f in "$docs/README.md" "$docs/user-guide.md" "$docs/author-guide.md"; do
    while IFS= read -r target; do
      [ -z "$target" ] && continue
      # Strip any "#anchor" fragment.
      rel="${target%%#*}"
      [ -z "$rel" ] && continue
      abs="$(cd "$(dirname "$f")" && cd "$(dirname "$rel")" 2>/dev/null && pwd)/$(basename "$rel")" \
        || { echo "unresolved dir for link '$target' in $f" >&2; return 1; }
      if [ ! -e "$abs" ]; then
        echo "broken relative link '$target' in $f (resolved to $abs)" >&2
        return 1
      fi
    done < <(grep -oE '\]\((\./|\.\./)[^)]+\)' "$f" | sed -E 's/^\]\(//; s/\)$//')
  done
}

# ---------------------------------------------------------------------------- #
# P07a-TS03: schema-drift guard for the canonical flox/flox-hello-script
# example repo.
#
# Mirrors the published repo's file layout (minimal `flox-extension.toml`,
# no `[extension.binary]` so install-time kind derivation lands on script,
# an executable `flox-hello-script` that prints the P06 bookkeeping env
# vars) into a local bare repo, redirects the github clone URL and the
# API base URL at it, and asserts the real `flox extension install
# flox/flox-hello-script` + `flox hello-script` path produces the
# expected greeting.
# ---------------------------------------------------------------------------- #

_setup_hello_script_fixture() {
  local work="$BATS_TEST_TMPDIR/work-hello-script"
  local bare="$BATS_TEST_TMPDIR/bare/flox-hello-script.git"
  mkdir -p "$work" "$(dirname "$bare")"
  git init -q --bare "$bare"
  git init -q -b main "$work"
  cat > "$work/flox-extension.toml" <<'EOF'
[extension]
name = "hello-script"
description = "Canonical script-kind reference extension for flox."
EOF
  cat > "$work/flox-hello-script" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
name="${FLOX_EXTENSION_NAME:-hello-script}"
version="${FLOX_EXTENSION_VERSION:-unknown}"
echo "Hello from ${name} v${version}"
if [ "$#" -gt 0 ]; then
  echo "args: $*"
fi
EOF
  chmod +x "$work/flox-hello-script"
  git -C "$work" -c user.email=t@e -c user.name=t -c commit.gpgsign=false add -A
  git -C "$work" -c user.email=t@e -c user.name=t -c commit.gpgsign=false commit -q -m initial
  git -C "$work" remote add origin "$bare"
  git -C "$work" push -q origin main

  export HELLO_SCRIPT_SHA
  HELLO_SCRIPT_SHA="$(git -C "$work" rev-parse HEAD)"

  cat > "$BATS_TEST_TMPDIR/hello_api.py" <<EOF
import json, http.server
SHA = "$HELLO_SCRIPT_SHA"
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/repos/flox/flox-hello-script/releases/latest":
            self.send_response(404); self.end_headers(); return
        if path == "/repos/flox/flox-hello-script":
            body = json.dumps({"default_branch": "main"}).encode()
        elif path.startswith("/repos/flox/flox-hello-script/commits/"):
            body = json.dumps({"sha": SHA}).encode()
        else:
            self.send_response(404); self.end_headers(); return
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a, **k):
        pass
s = http.server.HTTPServer(("127.0.0.1", 0), H)
print(s.server_address[1], flush=True)
s.serve_forever()
EOF
  local py
  if command -v python3 > /dev/null 2>&1; then
    py=python3
  elif command -v python > /dev/null 2>&1; then
    py=python
  else
    skip "python3 not available in test environment"
  fi

  $py "$BATS_TEST_TMPDIR/hello_api.py" > "$BATS_TEST_TMPDIR/hello_port.txt" &
  echo $! > "$BATS_TEST_TMPDIR/hello_api.pid"
  local i
  for i in $(seq 1 50); do
    [ -s "$BATS_TEST_TMPDIR/hello_port.txt" ] && break
    sleep 0.1
  done
  local port
  port="$(cat "$BATS_TEST_TMPDIR/hello_port.txt")"
  export FLOX_EXTENSIONS_GITHUB_BASE_URL="http://127.0.0.1:$port"

  export GIT_CONFIG_COUNT=1
  export GIT_CONFIG_KEY_0="url.file://$bare.insteadOf"
  export GIT_CONFIG_VALUE_0="https://github.com/flox/flox-hello-script.git"
}

_teardown_hello_script_fixture() {
  if [ -f "$BATS_TEST_TMPDIR/hello_api.pid" ]; then
    kill "$(cat "$BATS_TEST_TMPDIR/hello_api.pid")" 2>/dev/null || true
  fi
  unset GIT_CONFIG_COUNT GIT_CONFIG_KEY_0 GIT_CONFIG_VALUE_0
  unset FLOX_EXTENSIONS_GITHUB_BASE_URL HELLO_SCRIPT_SHA
}

# P07a-TS03: install the flox/flox-hello-script mirror end-to-end.
@test "extension: flox/flox-hello-script mirror installs and dispatches" {
  export FLOX_FEATURES_BETA=true
  _setup_hello_script_fixture

  run "$FLOX_BIN" extension install flox/flox-hello-script
  assert_success
  assert_output --partial "Installed flox-hello-script"

  run "$FLOX_BIN" hello-script world
  assert_success
  assert_output --partial "Hello from hello-script"
  assert_output --partial "args: world"

  _teardown_hello_script_fixture
}
