#!/usr/bin/env bash
#
# run-tests.sh — test suite for the Flox "virtual sandbox" libraries.
#
# This drives two complementary layers of testing:
#
#   1. threadtest — links the closure policy directly and hammers in_closure()
#      from many threads, using a behavioral oracle to catch the shared-buffer
#      data races that previously corrupted answers (and crashed on macOS).
#
#   2. sandbox_probe — a separate process with the sandbox library injected via
#      the platform loader mechanism (DYLD_INSERT_LIBRARIES on macOS, LD_PRELOAD
#      on Linux). This validates the real interception path end to end:
#        - sandbox=off    : no interference
#        - sandbox=warn   : out-of-closure accesses warn but succeed
#        - sandbox=enforce: out-of-closure accesses are blocked / fatal
#        - threaded storm : the interceptors survive concurrent load
#
# The fixtures are built from real /nix/store paths, so this must run on a host
# with a populated Nix store (true for every Flox build/dev environment).
#
# Exit status is 0 only if every check passes.

set -u

# ----------------------------------------------------------------------------
# Locate ourselves and the artifacts under test. The script lives in
# package-builder/tests; the libraries and (Makefile-built) probe binaries live
# one level up in package-builder.
# ----------------------------------------------------------------------------
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
cd "$root"

# Per-OS choice of injected library, loader variable, and ECONNREFUSED value.
# ECONNREFUSED is the errno the sandbox sets when it refuses an out-of-policy
# connect; its numeric value is platform-specific (61 on Darwin, 111 on Linux),
# and the connect probe prints "errno=<n>", so the test asserts the right one.
case "$(uname -s)" in
  Darwin)
    sandbox_lib="$root/libsandbox.dylib"
    preload_var="DYLD_INSERT_LIBRARIES"
    ECONNREFUSED_NO=61
    ;;
  Linux)
    sandbox_lib="$root/libsandbox.so"
    preload_var="LD_PRELOAD"
    ECONNREFUSED_NO=111
    ;;
  *)
    echo "run-tests.sh: unsupported OS $(uname -s)" >&2
    exit 2
    ;;
esac

if [[ ! -e "$sandbox_lib" ]]; then
  echo "run-tests.sh: $sandbox_lib not built; run 'make' first" >&2
  exit 2
fi

# Test bookkeeping.
tests_run=0
tests_failed=0
pass() { printf 'ok   - %s\n' "$1"; tests_run=$((tests_run + 1)); }
fail() {
  printf 'FAIL - %s\n' "$1"
  [[ -n "${2:-}" ]] && printf '       %s\n' "$2"
  tests_run=$((tests_run + 1))
  tests_failed=$((tests_failed + 1))
}

# ----------------------------------------------------------------------------
# Build a minimal $FLOX_ENV fixture from real store paths, WITHOUT using `ls`
# (which is frequently aliased to append a classifying trailing slash, and a
# trailing slash in requisites.txt is exactly the kind of input we must handle
# robustly). We pick:
#   in_store  : a store directory that contains at least one regular file
#   in_file   : that regular file (must be reported IN closure)
#   out_store : a different store directory containing a file
#   out_file  : that file (NOT listed in requisites -> out of closure)
# ----------------------------------------------------------------------------
fixture="$(mktemp -d "${TMPDIR:-/tmp}/flox-sandbox-tests.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

in_store="" in_file="" out_store="" out_file=""
for d in /nix/store/*/; do
  d="${d%/}"                 # strip the glob's trailing slash
  [[ -d "$d" ]] || continue
  f="$(find "$d" -type f -print -quit 2>/dev/null)"
  [[ -n "$f" ]] || continue  # need a directory with a real file inside
  if [[ -z "$in_store" ]]; then
    in_store="$d"; in_file="$f"
  else
    out_store="$d"; out_file="$f"; break
  fi
done

if [[ -z "$in_file" || -z "$out_file" ]]; then
  echo "run-tests.sh: could not find two usable store paths for fixtures" >&2
  exit 2
fi

# requisites.txt lists ONLY the in-closure store object.
printf '%s\n' "$in_store" > "$fixture/requisites.txt"

# Allow the source/working directory so that incidental startup opens (the
# loader touching argv[0]'s directory, etc.) do not trip enforce mode. This
# mirrors flox-build.mk, which sets FLOX_SRC_DIR=$(PWD) for real builds.
allow_dirs="$root"

# Helper: run sandbox_probe with the library injected and a given mode.
# Usage: run_probe <mode> <probe-args...>
run_probe() {
  local mode="$1"; shift
  env "$preload_var=$sandbox_lib" \
      FLOX_ENV="$fixture" \
      FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" \
      FLOX_VIRTUAL_SANDBOX="$mode" \
      "$root/tests/sandbox_probe" "$@"
}

echo "# fixture: $fixture"
echo "# in_file : $in_file"
echo "# out_file: $out_file"
echo

# ----------------------------------------------------------------------------
# Layer 1: thread-safety regression (behavioral oracle).
# ----------------------------------------------------------------------------
if "$root/tests/threadtest" >/tmp/threadtest.$$ 2>&1; then
  pass "threadtest: in_closure() race-free under concurrency"
else
  fail "threadtest: mismatches detected (data race)" "$(tail -2 /tmp/threadtest.$$)"
fi
rm -f /tmp/threadtest.$$

# ----------------------------------------------------------------------------
# Layer 2: real interception semantics.
# ----------------------------------------------------------------------------

# off: out-of-closure open must succeed with no warning.
out="$(run_probe off open "$out_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" != *"WARNING"* ]]; then
  pass "off: out-of-closure open allowed, silent"
else
  fail "off: expected silent OPEN_OK" "$out"
fi

# warn: out-of-closure open succeeds AND warns about that path.
out="$(run_probe warn open "$out_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" == *"$out_file is not in the sandbox"* ]]; then
  pass "warn: out-of-closure open warned but allowed"
else
  fail "warn: expected warning + OPEN_OK for out-of-closure" "$out"
fi

# warn: in-closure open succeeds with NO warning about that path.
out="$(run_probe warn open "$in_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" != *"$in_file is not in the sandbox"* ]]; then
  pass "warn: in-closure open allowed, silent"
else
  fail "warn: in-closure open should not warn" "$out"
fi

# enforce: in-closure open succeeds and the process exits cleanly.
out="$(run_probe enforce open "$in_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* ]]; then
  pass "enforce: in-closure open allowed"
else
  fail "enforce: in-closure open should succeed (rc=$rc)" "$out"
fi

# enforce: out-of-closure open is fatal (nonzero exit, ERROR about that path).
out="$(run_probe enforce open "$out_file" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"$out_file is not in the sandbox"* ]]; then
  pass "enforce: out-of-closure open blocked (rc=$rc)"
else
  fail "enforce: out-of-closure open should be fatal" "$out"
fi

# Message attribution: every SANDBOX line carries [exe:pid] so a report can
# be traced to the process that triggered it (a bare PID is useless once the
# process exits). The probe binary is sandbox_probe, so the ERROR line from
# the enforce denial above must name it alongside the PID.
if grep -Eq 'SANDBOX ERROR\[sandbox_probe:[0-9]+\]:' <<<"$out"; then
  pass "enforce: denial line attributes [exe:pid] (sandbox_probe)"
else
  fail "enforce: ERROR line should carry [exe:pid] attribution" "$out"
fi

# enforce: a $HOME dotfile is PERMITTED (not blocked) but warned, even under
# enforce. The test HOME is created under the real $HOME, which is not part of
# any built-in allow-dir prefix, so the access exercises the home-dotfile path
# rather than (say) the /tmp directory allow.
home_dir="$(mktemp -d "$HOME/flox-sandbox-tests-home.XXXXXX")"
printf 'x' > "$home_dir/.dotfile"
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    HOME="$home_dir" "$root/tests/sandbox_probe" open "$home_dir/.dotfile" 2>&1)"; rc=$?
rm -rf "$home_dir"
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* \
      && "$out" == *"permitted as a \$HOME dotfile"* ]]; then
  pass "enforce: \$HOME dotfile allowed but warned"
else
  fail "enforce: \$HOME dotfile should be allowed with a warning" "$out"
fi

# enforce: a directory access (listing) is "looking around", not consuming
# out-of-closure contents, so it is permitted but warned even under enforce.
# out_store is an out-of-closure store directory.
out="$(run_probe enforce open "$out_store" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" == *"directory listing"* ]]; then
  pass "enforce: out-of-closure directory listing allowed but warned"
else
  fail "enforce: directory listing should be allowed with a warning" "$out"
fi

# enforce: open(O_DIRECTORY) on an out-of-closure regular file is a path
# probe, not a content read (the kernel returns ENOTDIR regardless). It must
# NOT be fatal under enforce — the sandbox should warn and permit it.
# This exercises the in_dir_probe path that Node.js and similar runtimes
# trigger during module/path resolution.
out="$(run_probe enforce open-dir "$out_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"directory probe"* ]]; then
  pass "enforce: O_DIRECTORY open on out-of-closure file is warned, not fatal"
else
  fail "enforce: O_DIRECTORY open should warn-but-allow under enforce (rc=$rc)" "$out"
fi

# readlinkat is intercepted too, but treated like a directory listing: reading
# a symlink is "looking around", so it is warned-but-permitted even under
# enforce (never fatal). The symlink resolves (via realpath) to out_file.
link="$fixture/outlink"; ln -sf "$out_file" "$link"
out="$(run_probe off readlink "$link" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"READLINK_OK"* && "$out" != *"symlink read"* ]]; then
  pass "off: readlinkat passes through, silent"
else
  fail "off: readlinkat should succeed silently" "$out"
fi
out="$(run_probe enforce readlink "$link" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"READLINK_OK"* && "$out" == *"symlink read"* ]]; then
  pass "enforce: readlinkat of out-of-closure target permitted but warned"
else
  fail "enforce: readlinkat should warn-but-allow under enforce (rc=$rc)" "$out"
fi

# readlink() (non-at POSIX form) is intercepted and treated the same way.
# sandbox_probe readlink-fn calls plain readlink() rather than readlinkat().
out="$(run_probe off readlink-fn "$link" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"READLINK_OK"* && "$out" != *"symlink read"* ]]; then
  pass "off: readlink (non-at) passes through, silent"
else
  fail "off: readlink-fn should succeed silently" "$out"
fi
out="$(run_probe enforce readlink-fn "$link" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"READLINK_OK"* && "$out" == *"symlink read"* ]]; then
  pass "enforce: readlink (non-at) of out-of-closure target permitted but warned"
else
  fail "enforce: readlink-fn should warn-but-allow under enforce (rc=$rc)" "$out"
fi

# __readlink_chk coverage via a real tool. Coreutils 'readlink' (and 'ls -la')
# are compiled with -D_FORTIFY_SOURCE=2 and bind to __readlink_chk rather than
# plain readlink; without a specific interceptor for that symbol, symlink reads
# in those tools would silently bypass the sandbox. __readlink_chk is a
# glibc-specific symbol, so this check only applies on Linux.
readlink_bin="$(command -v readlink 2>/dev/null || true)"
if [[ "$(uname -s)" == "Linux" && -n "$readlink_bin" ]]; then
  out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
      FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=warn \
      "$readlink_bin" "$link" 2>&1)"
  if [[ "$out" == *"$out_file"* && "$out" == *"symlink read"* ]]; then
    pass "warn: __readlink_chk via real tool (readlink) is intercepted"
  else
    fail "warn: readlink's __readlink_chk was not flagged — __readlink_chk interception gap" "$out"
  fi
else
  echo "skip - not Linux or 'readlink' not on PATH; __readlink_chk is glibc-only"
fi

# The directory-listing warning is de-duplicated per resolved path: a build that
# lists the same out-of-closure directory many times in one process must get a
# single warning, not one per access. Drive 8 opens of out_store from one
# process (storm, 1 thread) and assert exactly one "directory listing" line.
out="$(run_probe warn storm 1 8 "$out_store" 2>&1)"; rc=$?
n="$(grep -c "directory listing" <<<"$out")"
if [[ $rc -eq 0 && "$n" -eq 1 ]]; then
  pass "warn: repeated directory listing warns once (de-duplicated)"
else
  fail "warn: directory-listing warning should appear once, saw $n" "$out"
fi

# The same per-path de-duplication applies to the warn-mode "not in the sandbox"
# file warning: reading the same out-of-closure file repeatedly yields a single
# warning. out_file is an out-of-closure regular file.
out="$(run_probe warn storm 1 8 "$out_file" 2>&1)"; rc=$?
n="$(grep -c "is not in the sandbox" <<<"$out")"
if [[ $rc -eq 0 && "$n" -eq 1 ]]; then
  pass "warn: repeated out-of-closure file read warns once (de-duplicated)"
else
  fail "warn: out-of-closure file warning should appear once, saw $n" "$out"
fi

# A relative/symlinked path is reported with its resolved realpath in
# parentheses. Opening ".." (a directory) from $root resolves to the repo root,
# so the message should read ".. (<realpath>)".
out="$(run_probe warn open .. 2>&1)"; rc=$?
if [[ "$out" == *".. ("* ]]; then
  pass "warn: relative path reported with resolved realpath in parentheses"
else
  fail "warn: expected '.. (<realpath>)' in the warning" "$out"
fi

# A FLOX_SANDBOX_ALLOW glob (from the manifest's sandbox-allow) silently permits
# a matched out-of-closure FILE even under enforce. out_file is out of closure;
# a recursive glob over its store directory matches it.
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_SANDBOX_ALLOW="$out_store/**" \
    FLOX_VIRTUAL_SANDBOX=enforce "$root/tests/sandbox_probe" open "$out_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" != *"not in the sandbox"* ]]; then
  pass "enforce: FLOX_SANDBOX_ALLOW glob silently permits a matched file"
else
  fail "enforce: sandbox-allow glob should silently permit the matched file" "$out"
fi

# fopen() coverage via a REAL tool. coreutils `sum` reads files with fopen()
# rather than open()/openat() — and on macOS it binds the fopen$DARWIN_EXTSN
# variant, a symbol distinct from plain fopen. This is a regression guard for
# that interception path: a synthetic probe calling plain fopen() would NOT
# have caught the macOS variant gap, so we drive the actual tool. In warn mode
# the out-of-closure read is allowed but must be flagged, which only happens if
# the fopen() was intercepted.
sum_bin="$(command -v sum 2>/dev/null || true)"
if [[ -n "$sum_bin" ]]; then
  out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
      FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=warn \
      "$sum_bin" /etc/hosts 2>&1)"
  if [[ "$out" == *"/etc/hosts"* && "$out" == *"not in the sandbox"* ]]; then
    pass "warn: fopen() via a real tool (sum) is intercepted"
  else
    fail "warn: sum's fopen(/etc/hosts) was not flagged — fopen interception gap" "$out"
  fi
else
  echo "skip - 'sum' not on PATH; cannot exercise real-tool fopen() coverage"
fi

# ----------------------------------------------------------------------------
# Layer 2c: interactive prompt broker (Phase 2). With FLOX_SANDBOX_PROMPT_SOCKET
# set, an out-of-closure access is referred to the broker instead of being
# warned/blocked outright. We drive the libsandbox prompt client against the
# fixed-reply mock broker and check both decisions under enforce.
# ----------------------------------------------------------------------------

# Run a probe in prompt mode with the mock broker replying $1, opening $out_file
# (out of closure). Echoes "<rc>|<stdout+stderr>".
run_with_broker() {
  local reply="$1"
  local sock; sock="$(mktemp -u "${TMPDIR:-/tmp}/flox-prompt.XXXXXX.sock")"
  "$root/tests/mock_prompt_broker" "$sock" "$reply" >/dev/null 2>&1 &
  local broker_pid=$!
  # Wait (briefly) for the socket to appear.
  for _ in $(seq 1 50); do [[ -S "$sock" ]] && break; sleep 0.05; done
  local out rc
  out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
      FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=prompt \
      FLOX_SANDBOX_PROMPT_SOCKET="$sock" \
      "$root/tests/sandbox_probe" open "$out_file" 2>&1)"; rc=$?
  kill "$broker_pid" 2>/dev/null; wait "$broker_pid" 2>/dev/null
  rm -f "$sock"
  printf '%s|%s' "$rc" "$out"
}

# Broker "allow": the out-of-closure read is permitted, no error, the probe
# succeeds.
res="$(run_with_broker allow)"; rc="${res%%|*}"; out="${res#*|}"
if [[ "$rc" -eq 0 && "$out" == *"OPEN_OK"* && "$out" != *"not in the sandbox"* ]]; then
  pass "prompt: broker 'allow' permits an out-of-closure file"
else
  fail "prompt: broker 'allow' should permit the access (rc=$rc)" "$out"
fi

# Broker "deny": the access is refused (EACCES), so the probe's open() fails.
res="$(run_with_broker deny)"; rc="${res%%|*}"; out="${res#*|}"
if [[ "$rc" -ne 0 && "$out" == *"OPEN_FAIL"* && "$out" == *"denied by sandbox prompt"* ]]; then
  pass "prompt: broker 'deny' refuses an out-of-closure file"
else
  fail "prompt: broker 'deny' should refuse the access (rc=$rc)" "$out"
fi

# No broker configured: prompt mode falls back to plain enforce (the access is
# fatal), which is what a non-interactive build gets.
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=prompt \
    "$root/tests/sandbox_probe" open "$out_file" 2>&1)"; rc=$?
if [[ "$rc" -ne 0 && "$out" == *"is not in the sandbox"* ]]; then
  pass "prompt: with no broker, falls back to enforce (blocks)"
else
  fail "prompt: no-broker should behave as enforce (rc=$rc)" "$out"
fi

# Mode gating: an enforce-mode build that happens to share the process-wide
# prompt socket env must NOT consult the broker (it would say allow); only
# prompt mode prompts. Start an allow-broker but run under enforce: the access
# is still blocked.
sock="$(mktemp -u "${TMPDIR:-/tmp}/flox-prompt.XXXXXX.sock")"
"$root/tests/mock_prompt_broker" "$sock" allow >/dev/null 2>&1 &
broker_pid=$!
for _ in $(seq 1 50); do [[ -S "$sock" ]] && break; sleep 0.05; done
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    FLOX_SANDBOX_PROMPT_SOCKET="$sock" \
    "$root/tests/sandbox_probe" open "$out_file" 2>&1)"; rc=$?
kill "$broker_pid" 2>/dev/null; wait "$broker_pid" 2>/dev/null; rm -f "$sock"
if [[ "$rc" -ne 0 && "$out" == *"is not in the sandbox"* ]]; then
  pass "enforce: ignores the prompt socket (only prompt mode consults the broker)"
else
  fail "enforce: must not consult the broker (rc=$rc)" "$out"
fi

# ----------------------------------------------------------------------------
# Layer 3: threaded interception storm (stability of the real interceptors).
# The OLD library crashed here on macOS (uninitialized mutex + shared buffers);
# the fixed library must run to completion. Mixed in/out/nonexistent paths
# exercise every branch. Warn mode keeps every open non-fatal.
# ----------------------------------------------------------------------------
out="$(run_probe warn storm 12 2000 "$in_file" "$out_file" /etc/hosts /no/such/path 2>/dev/null)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"STORM_OK"* ]]; then
  pass "storm: 12 threads survived concurrent interception"
else
  fail "storm: threaded interception did not complete cleanly (rc=$rc)" "$out"
fi

# ----------------------------------------------------------------------------
# Layer 4: prompt mode under an activation.
#
# In prompt mode, an out-of-policy access asks the broker over
# FLOX_SANDBOX_PROMPT_SOCKET for a verdict. sandbox_check_path() returns the
# broker's allow/deny; on deny the interceptor's errno=EACCES branch fires and
# the calling process sees a clean permission error (NOT aborted, unlike
# enforce). Under an ACTIVATION (FLOX_SANDBOX_ALLOW_FOREIGN_EXE set), a
# dead/absent broker fails closed: deny plus a distinct "SANDBOX ERROR ...
# prompt broker unreachable" receipt, rate-limited once per path. (A BUILD
# with no broker falls back to plain enforce — covered in Layer 2c.)
# ----------------------------------------------------------------------------

# Activation helper for this layer: run_probe plus the activation marker, so
# the no-broker path is the graceful fail-closed deny rather than build
# enforce.
run_probe_activation() {
  local mode="$1"; shift
  env "$preload_var=$sandbox_lib" \
      FLOX_ENV="$fixture" \
      FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" \
      FLOX_VIRTUAL_SANDBOX="$mode" \
      FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1 \
      "$root/tests/sandbox_probe" "$@"
}

# prompt + no socket env → fail-closed deny. No FLOX_SANDBOX_PROMPT_SOCKET is
# set, so the RPC has nothing to connect to. The read fails with EACCES (the
# probe prints OPEN_FAIL — no crash, no exit(1)), and exactly one fail-closed
# "SANDBOX ERROR ... prompt broker unreachable" line appears for the path (not
# the two-line DENIED receipt, which is reserved for a real deny verdict).
out="$(run_probe_activation prompt open "$out_file" 2>&1)"; rc=$?
error_n="$(grep -c "prompt broker unreachable" <<<"$out")"
if [[ $rc -ne 0 && "$out" == *"OPEN_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"denying read of $out_file (fail-closed"* \
      && "$error_n" -eq 1 && "$out" != *"SANDBOX DENIED"* ]]; then
  pass "prompt: no socket → fail-closed EACCES + SANDBOX ERROR once per path"
else
  fail "prompt: expected EACCES (errno=13) + fail-closed SANDBOX ERROR (rc=$rc, ERROR lines=$error_n)" "$out"
fi

# Dotfile flip. Under a prompt-mode ACTIVATION the $HOME-dotfile carve-out is
# skipped, so a read of an out-of-closure $HOME dotfile is routed through the
# prompt flow and DENIED (here with no socket, that deny is the fail-closed
# EACCES). Under enforce the same read is PERMITTED with the existing "$HOME
# dotfile" warn line. The dotfile lives under a temp HOME that is not an
# allow-dir prefix, so the access genuinely exercises the home-dotfile branch
# rather than a directory allow.
home_dir="$(mktemp -d "$HOME/flox-sandbox-tests-home.XXXXXX")"
printf 'x' > "$home_dir/.fakerc"

out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=prompt \
    FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1 \
    HOME="$home_dir" "$root/tests/sandbox_probe" open "$home_dir/.fakerc" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"OPEN_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"denying read of $home_dir/.fakerc (fail-closed"* ]]; then
  pass "prompt: \$HOME dotfile read denied (carve-out skipped for activations)"
else
  fail "prompt: \$HOME dotfile should be denied under a prompt activation (rc=$rc)" "$out"
fi

# A prompt-mode BUILD (no FLOX_SANDBOX_ALLOW_FOREIGN_EXE) keeps the dotfile
# carve-out, like every other build level — the skip is an activation-only
# threat-model decision.
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=prompt \
    HOME="$home_dir" "$root/tests/sandbox_probe" open "$home_dir/.fakerc" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* \
      && "$out" == *"permitted as a \$HOME dotfile"* ]]; then
  pass "prompt: \$HOME dotfile still permitted for a prompt-mode build"
else
  fail "prompt: \$HOME dotfile should keep the carve-out for builds (rc=$rc)" "$out"
fi

out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    HOME="$home_dir" "$root/tests/sandbox_probe" open "$home_dir/.fakerc" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* \
      && "$out" == *"permitted as a \$HOME dotfile"* ]]; then
  pass "enforce: same \$HOME dotfile still permitted (carve-out intact off-prompt)"
else
  fail "enforce: \$HOME dotfile should remain permitted under enforce (rc=$rc)" "$out"
fi
rm -rf "$home_dir"

# Golden stability: a NORMAL out-of-closure file (not a dotfile) must behave
# exactly as before under warn and enforce — prompt must not have perturbed the
# other levels. warn warns-but-permits; enforce is fatal with the same message.
out="$(run_probe warn open "$out_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" == *"$out_file is not in the sandbox"* ]]; then
  pass "warn: normal out-of-closure read unchanged by prompt addition"
else
  fail "warn: normal out-of-closure read should warn-but-permit (rc=$rc)" "$out"
fi
out="$(run_probe enforce open "$out_file" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"$out_file is not in the sandbox"* && "$out" != *"DENIED"* ]]; then
  pass "enforce: normal out-of-closure read still fatal, no DENIED receipt"
else
  fail "enforce: normal out-of-closure read should remain fatal (rc=$rc)" "$out"
fi

# ----------------------------------------------------------------------------
# Layer 4.1: prompt broker RPC (against a scripted fake broker).
#
# The real broker is a thread in the flox-activations executive. Here a tiny
# Python script (tests/fake-broker.py) binds the prompt socket and replies a
# canned line-protocol verdict, logging every request so we can assert the RPC
# count. These cases prove the libsandbox prompt client end to end: an
# allow-glob answer is cached so it makes zero further RPCs, a queued deny
# produces the two-line receipt (deduped), and the negative TTL expires so a
# later allow is picked up on retry.
#
# Skipped gracefully if python3 is unavailable (the rest of the suite, which
# needs no broker, still runs).
# ----------------------------------------------------------------------------

broker_pid=""
broker_log=""
broker_sock=""
broker_mode_file=""
broker_ready=""
stop_fake_broker() {
  if [[ -n "$broker_pid" ]]; then
    # SIGTERM lets the broker close its socket cleanly; fall back to SIGKILL.
    kill "$broker_pid" 2>/dev/null
    wait "$broker_pid" 2>/dev/null
    broker_pid=""
  fi
  # Belt-and-braces: remove the socket so a later start rebinds cleanly even if
  # a stray broker lingered.
  [[ -n "$broker_sock" ]] && rm -f "$broker_sock"
}
# Extend the existing EXIT cleanup (which removes $fixture) so no broker
# outlives the suite even if a case bails early. Re-set the trap to do both.
trap 'stop_fake_broker; rm -rf "$fixture"' EXIT
# Start the fake broker in MODE (allow-scope|allow-file|deny), optionally with
# a --scope glob. Waits for it to print READY (socket bound) before returning.
# Sets broker_sock / broker_log / broker_mode_file for the caller and the probe.
#
# The broker is a plain background job with stdin/stdout/stderr redirected to
# files (NOT inherited from the test's stdout). This is deliberate: a broker
# that inherited the test's stdout pipe would hold it open after the test
# exits, so the `make tests | ...` pipe would never see EOF and make would
# appear to hang. Redirecting detaches it, and $! gives the real PID to reap.
start_fake_broker() {
  local mode="$1"; local scope="${2:-}"
  broker_sock="$fixture/broker.$$.sock"
  broker_log="$fixture/broker.$$.log"
  broker_mode_file="$fixture/broker.$$.mode"
  broker_ready="$fixture/broker.$$.ready"
  rm -f "$broker_sock" "$broker_log" "$broker_mode_file" "$broker_ready"
  local args=(--socket "$broker_sock" --log "$broker_log" --mode "$mode"
              --mode-file "$broker_mode_file")
  [[ -n "$scope" ]] && args+=(--scope "$scope")
  python3 "$here/fake-broker.py" "${args[@]}" </dev/null >"$broker_ready" 2>&1 &
  broker_pid=$!
  # Poll the readiness file for the READY line (socket bound) before returning.
  local waited=0
  while [[ $waited -lt 100 ]]; do
    if grep -q READY "$broker_ready" 2>/dev/null; then
      return 0
    fi
    # Bail if the broker died before becoming ready.
    kill -0 "$broker_pid" 2>/dev/null || return 1
    sleep 0.1
    waited=$((waited + 1))
  done
  return 1
}
# Run the probe as an activation with the broker socket configured.
run_probe_prompt_socket() {
  env "$preload_var=$sandbox_lib" \
      FLOX_ENV="$fixture" \
      FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" \
      FLOX_VIRTUAL_SANDBOX=prompt \
      FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1 \
      FLOX_SANDBOX_PROMPT_SOCKET="$broker_sock" \
      "$root/tests/sandbox_probe" "$@"
}

if ! command -v python3 >/dev/null 2>&1; then
  pass "prompt broker RPC tests skipped (python3 unavailable)"
else
  # The broker cases need paths that are genuinely OUT of policy so the access
  # reaches the prompt tail and RPCs. The fixture lives under $TMPDIR / /tmp,
  # both of which are built-in allow-dirs, so a path there is permitted WITHOUT
  # any RPC. $HOME is not an allow-dir (only $HOME/.dotfiles get a carve-out,
  # and that is skipped under a prompt activation), so a regular subdir of
  # $HOME is out of policy. Use one such dir for all broker cases and remove it
  # at the end.
  prompt_home="$(mktemp -d "$HOME/flox-sandbox-prompt-tests.XXXXXX")"

  # Case 2: allow scope glob → open succeeds; a second open UNDER the scope
  # makes ZERO further RPCs (the C scope-verdict cache answers it). We probe two
  # sibling files under a dir and grant the dir glob.
  scope_dir="$prompt_home/scope_probe"
  mkdir -p "$scope_dir"
  printf 'a' > "$scope_dir/a.txt"
  printf 'b' > "$scope_dir/b.txt"
  if start_fake_broker allow-scope "$scope_dir/*"; then
    out1="$(run_probe_prompt_socket open "$scope_dir/a.txt" 2>&1)"; rc1=$?
    out2="$(run_probe_prompt_socket open "$scope_dir/b.txt" 2>&1)"; rc2=$?
    stop_fake_broker
    if [[ $rc1 -eq 0 && "$out1" == *"OPEN_OK"* && $rc2 -eq 0 \
          && "$out2" == *"OPEN_OK"* ]]; then
      pass "prompt: broker allow-scope verdict permits the open"
    else
      fail "prompt: allow-scope should permit (rc1=$rc1 rc2=$rc2)" "$out1"$'\n'"$out2"
    fi
  else
    fail "prompt: could not start fake broker (allow-scope)"
    stop_fake_broker
  fi

  # Case 2b: zero further RPCs within one process. The storm probe opens the
  # same in-scope path many times in one process; the client's allow-glob
  # cache must answer all but the first, so the broker logs exactly ONE
  # request.
  if start_fake_broker allow-scope "$scope_dir/*"; then
    out="$(run_probe_prompt_socket storm 1 8 "$scope_dir/a.txt" 2>&1)"; rc=$?
    req_n="$(wc -l <"$broker_log" | tr -d ' ')"
    stop_fake_broker
    if [[ $rc -eq 0 && "$req_n" -eq 1 ]]; then
      pass "prompt: second open under an allowed scope makes zero further RPCs"
    else
      fail "prompt: scope cache should collapse 8 opens to 1 RPC (rc=$rc, RPCs=$req_n)" "$out"
    fi
  else
    fail "prompt: could not start fake broker (allow-scope 2b)"
    stop_fake_broker
  fi

  # Case 3: deny verdict → EACCES + two-line receipt, deduped on repeat. The
  # storm probe opens the same out-of-policy path 4 times in one process; the
  # receipt is printed once (deduped per path), and the read fails with EACCES.
  if start_fake_broker deny; then
    out="$(run_probe_prompt_socket open "$out_file" 2>&1)"; rc=$?
    denied_n="$(grep -c "SANDBOX DENIED" <<<"$out")"
    # Repeat in one process to prove receipt dedup: the storm worker opens the
    # same path repeatedly but the receipt appears at most twice (the two
    # DENIED lines for one path), never per-open.
    out_storm="$(run_probe_prompt_socket storm 1 4 "$out_file" 2>&1)"
    denied_storm="$(grep -c "SANDBOX DENIED" <<<"$out_storm")"
    stop_fake_broker
    if [[ $rc -ne 0 && "$out" == *"OPEN_FAIL"* && "$out" == *"errno=13"* \
          && "$out" == *"read $out_file (not in policy)"* \
          && "$out" == *"queued as req"* && "$denied_n" -eq 2 \
          && "$denied_storm" -eq 2 ]]; then
      pass "prompt: broker deny → EACCES + two-line receipt, deduped on repeat"
    else
      fail "prompt: deny should EACCES + 2-line receipt deduped (rc=$rc, DENIED=$denied_n, storm=$denied_storm)" "$out"
    fi
  else
    fail "prompt: could not start fake broker (deny)"
    stop_fake_broker
  fi

  # Case 4: negative TTL expiry, within ONE process so the C deny cache is in
  # play. The open-twice probe opens the path (denied, cached for 2s), sleeps
  # past the TTL, then opens again. Mid-sleep the harness flips the live broker
  # from deny to allow. The first open fails (EACCES, cached deny); the second
  # open — past the TTL — re-asks the now-allowing broker and succeeds, proving
  # the negative cache entry expired rather than pinning the path closed.
  if start_fake_broker deny; then
    # Launch the two-open probe in the background; flip the broker during its
    # sleep so the second open (after the TTL) sees allow.
    out_ttl_file="$fixture/ttl.$$.out"
    run_probe_prompt_socket open-twice "$out_file" 3 >"$out_ttl_file" 2>&1 &
    probe_pid=$!
    sleep 0.5
    printf 'allow-file' > "$broker_mode_file"  # broker now allows the retry
    wait "$probe_pid"; rc_ttl=$?
    out_ttl="$(cat "$out_ttl_file")"
    stop_fake_broker
    if [[ "$out_ttl" == *"FIRST OPEN_FAIL"*"errno=13"* \
          && "$out_ttl" == *"SECOND OPEN_OK"* && $rc_ttl -eq 0 ]]; then
      pass "prompt: deny then allow-after-TTL → retry succeeds (negative TTL expiry)"
    else
      fail "prompt: TTL retry should succeed after broker flips to allow (rc=$rc_ttl)" "$out_ttl"
    fi
  else
    fail "prompt: could not start fake broker (TTL)"
    stop_fake_broker
  fi

  # Case 8: grants-dir write guard. The grants dir sits inside the project
  # allow-dir, so an access there is normally permitted with NO RPC. The guard
  # makes WRITES the exception: a write to an existing file under
  # FLOX_SANDBOX_GRANTS_DIR is routed through the prompt flow (so an agent cannot
  # silently edit its own future-session approvals), while a READ of the same
  # path stays quiet (no RPC). $fixture is under $TMPDIR, a built-in allow-dir,
  # so the grants dir there is genuinely in-policy — exactly the condition the
  # guard must override for writes only.
  guard_dir="$fixture/guard_grants"
  mkdir -p "$guard_dir"
  printf 'x' > "$guard_dir/grants.toml"
  if start_fake_broker deny; then
    # WRITE to the existing grants.toml: the guard forces the prompt flow even
    # though the path is under the allow-dir → broker denies (RPC made). `write`
    # opens an existing file O_WRONLY so it has a realpath and is classified as
    # a write (a `create` of a new file would take the create-parent path, not
    # the guard).
    out_w="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
        FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=prompt \
        FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1 \
        FLOX_SANDBOX_PROMPT_SOCKET="$broker_sock" FLOX_SANDBOX_GRANTS_DIR="$guard_dir" \
        "$root/tests/sandbox_probe" write "$guard_dir/grants.toml" 2>&1)"; rc_w=$?
    req_after_write="$(wc -l <"$broker_log" | tr -d ' ')"
    # READ of the same file: not a write, so the guard does not fire; the path
    # is inside the allow-dir, so it is permitted with NO further RPC.
    out_r="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
        FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=prompt \
        FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1 \
        FLOX_SANDBOX_PROMPT_SOCKET="$broker_sock" FLOX_SANDBOX_GRANTS_DIR="$guard_dir" \
        "$root/tests/sandbox_probe" open "$guard_dir/grants.toml" 2>&1)"; rc_r=$?
    req_after_read="$(wc -l <"$broker_log" | tr -d ' ')"
    stop_fake_broker
    if [[ $rc_w -ne 0 && "$out_w" == *"WRITE_FAIL"* && "$out_w" == *"errno=13"* \
          && "$req_after_write" -eq 1 \
          && $rc_r -eq 0 && "$out_r" == *"OPEN_OK"* \
          && "$req_after_read" -eq "$req_after_write" ]]; then
      pass "prompt: grants-dir write RPCs (guard fires), read does not"
    else
      fail "prompt: grants-dir guard should RPC writes but not reads (write rc=$rc_w RPCs=$req_after_write, read rc=$rc_r RPCs=$req_after_read)" "$out_w"$'\n'"$out_r"
    fi
  else
    fail "prompt: could not start fake broker (grants-dir guard)"
    stop_fake_broker
  fi

  rm -rf "$prompt_home"
fi

# ----------------------------------------------------------------------------
# Layer 4.5: foreign-executable exemption (FLOX_SANDBOX_ALLOW_FOREIGN_EXE).
#
# maybe_report_process_outside_closure() reports (warn) or aborts (enforce/pure)
# when the RUNNING EXECUTABLE is outside the closure — a build-reproducibility
# heuristic ("the wrong toolchain is active"). For an activation that heuristic
# is backwards: the whole point of a sandboxed activation is to run the user's
# shell and host tools (the coding agent, git, python) from OUTSIDE the closure
# while mediating only file/network ACCESS. FLOX_SANDBOX_ALLOW_FOREIGN_EXE makes
# the exe-identity check a no-op so the inner shell does not abort before the
# user's command runs.
#
# Driving this requires a probe whose OWN executable is outside every allow set
# and outside the closure. The regular probe lives under $root, which is an
# allow-dir, so it is always exempt and never reaches the exe check. We copy it
# into a directory that is NOT an allow-dir (a subdir of $root) and run it with
# allow-dirs narrowed to the fixture only, with TMPDIR/FLOX_SRC_DIR cleared so
# they cannot incidentally cover it.
# ----------------------------------------------------------------------------
foreign_dir="$root/tests/foreign-exe-probe.$$"
mkdir -p "$foreign_dir"
cp "$root/tests/sandbox_probe" "$foreign_dir/probe"

# Helper: run the foreign probe (exe outside all allow sets) with a given mode,
# narrow allow-dirs (fixture only), and an explicit FLOX_SANDBOX_ALLOW_FOREIGN_EXE
# value ("" leaves it unset). Usage: run_foreign <mode> <foreign-flag> <args...>
run_foreign() {
  local mode="$1"; local foreign="$2"; shift 2
  local -a foreign_env=()
  [[ -n "$foreign" ]] && foreign_env=(FLOX_SANDBOX_ALLOW_FOREIGN_EXE="$foreign")
  env -u TMPDIR -u FLOX_SRC_DIR "$preload_var=$sandbox_lib" \
      FLOX_ENV="$fixture" \
      FLOX_SANDBOX_ALLOW_DIRS="$fixture" \
      FLOX_VIRTUAL_SANDBOX="$mode" \
      "${foreign_env[@]}" \
      "$foreign_dir/probe" "$@"
}

# Baseline (no flag): the existing fatal behaviour is unchanged. Under enforce a
# foreign-exe process that touches an out-of-policy path aborts at the exe check
# with "process executable ... is not in the sandbox".
out="$(run_foreign enforce "" open "$out_file" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"process executable"* \
      && "$out" == *"is not in the sandbox"* ]]; then
  pass "enforce: foreign exe still fatal without FLOX_SANDBOX_ALLOW_FOREIGN_EXE"
else
  fail "enforce: foreign-exe check should remain fatal when the flag is unset (rc=$rc)" "$out"
fi

# With the flag set, the exe-identity check is skipped: a foreign-exe process
# that reads an IN-POLICY (in-closure) file succeeds, with no "process
# executable ... is not in the sandbox" abort.
out="$(run_foreign enforce 1 open "$in_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" != *"process executable"* ]]; then
  pass "enforce: FLOX_SANDBOX_ALLOW_FOREIGN_EXE lets a foreign exe read in-policy"
else
  fail "enforce: foreign exe + flag should read in-policy without the exe abort (rc=$rc)" "$out"
fi

# The flag changes ONLY the exe-identity check: an out-of-policy FILE read from
# the same foreign-exe process is still denied (fatal under enforce), and the
# message is the per-file deny, not the exe abort.
out="$(run_foreign enforce 1 open "$out_file" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"$out_file is not in the sandbox"* \
      && "$out" != *"process executable"* ]]; then
  pass "enforce: flag exempts the exe but still denies an out-of-policy file read"
else
  fail "enforce: out-of-policy file read should still be denied with the flag set (rc=$rc)" "$out"
fi

# prompt + flag: the inner shell no longer exit(1)s on the foreign exe. A
# foreign exe reading an OUT-OF-POLICY file gets the graceful prompt deny
# (EACCES) — never the exe abort — so an activation completes past the shell.
# run_foreign sets no socket, so the deny here is the fail-closed form.
out="$(run_foreign prompt 1 open "$out_file" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"OPEN_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"denying read of $out_file (fail-closed"* \
      && "$out" != *"process executable"* ]]; then
  pass "prompt: flag exempts the foreign exe; out-of-policy read still EACCES-denied"
else
  fail "prompt: foreign exe + flag should EACCES-deny the file, not abort on the exe (rc=$rc)" "$out"
fi

rm -rf "$foreign_dir"

# ----------------------------------------------------------------------------
# Layer 4.6: activation policy hardening (sensitive set + write-create guard).
#
# These cover the activation-gated correctness fixes derived from the DX
# assessment. All are gated on FLOX_SANDBOX_ALLOW_FOREIGN_EXE (the activation
# signal): a build never sets it, so build behaviour stays byte-identical (the
# warn/enforce/pure goldens above are the regression guard for that).
#
#   sensitive set : credentials (~/.ssh, ~/.aws, ~/.netrc, **/.env, ...) are
#                   denied even under enforce, BEFORE the $HOME-dotfile carve-out
#                   — while a non-sensitive dotfile (~/.gitconfig) is still
#                   permitted, and an explicit FLOX_SANDBOX_ALLOW grant of the
#                   sensitive path overrides back to allow.
#   create guard  : a new-file write is judged by its parent directory's policy:
#                   under an in-policy dir it succeeds; outside it is denied.
#                   WITHOUT the activation flag (build mode), both creates are
#                   allowed, unchanged.
#   /nix/store    : a store read is permitted (DX-1 is seed-driven, but the
#                   engine also recognizes the store prefix for the create
#                   guard's parent check; a store read via an allow-dir is the
#                   equivalent engine-level assertion).
# ----------------------------------------------------------------------------

# Helper: run the probe as if inside an activation — the foreign-exe flag set,
# plus an explicit $HOME under the real home (so the home-dotfile / sensitive
# branches engage rather than an allow-dir prefix). Usage:
#   run_activation <mode> <home_dir> [extra VAR=VAL ...] -- <probe-args...>
run_activation() {
  local mode="$1"; local home_dir="$2"; shift 2
  local -a extra=()
  while [[ "$1" != "--" ]]; do extra+=("$1"); shift; done
  shift # consume "--"
  env "$preload_var=$sandbox_lib" \
      FLOX_ENV="$fixture" \
      FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" \
      FLOX_VIRTUAL_SANDBOX="$mode" \
      FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1 \
      HOME="$home_dir" \
      "${extra[@]}" \
      "$root/tests/sandbox_probe" "$@"
}

# Build a fake $HOME under the real home (not an allow-dir prefix) holding a
# sensitive file (~/.ssh/known_hosts) and a non-sensitive dotfile (~/.gitconfig).
sens_home="$(mktemp -d "$HOME/flox-sandbox-tests-home.XXXXXX")"
mkdir -p "$sens_home/.ssh"
printf 'x' > "$sens_home/.ssh/known_hosts"
printf 'x' > "$sens_home/.gitconfig"

# enforce + activation: a sensitive path (~/.ssh/known_hosts) is DENIED even
# though it is a $HOME dotfile — the sensitive set fires before the carve-out.
# The message names it as sensitive.
out="$(run_activation enforce "$sens_home" -- open "$sens_home/.ssh/known_hosts" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"is not in the sandbox (sensitive)"* ]]; then
  pass "enforce+activation: sensitive \$HOME path denied (sensitive set)"
else
  fail "enforce+activation: ~/.ssh/known_hosts should be denied as sensitive (rc=$rc)" "$out"
fi

# enforce + activation: a NON-sensitive $HOME dotfile (~/.gitconfig) is still
# permitted via the dotfile carve-out — the sensitive set is narrow, not a
# blanket dotfile denial.
out="$(run_activation enforce "$sens_home" -- open "$sens_home/.gitconfig" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* \
      && "$out" == *"permitted as a \$HOME dotfile"* ]]; then
  pass "enforce+activation: non-sensitive dotfile still permitted (carve-out)"
else
  fail "enforce+activation: ~/.gitconfig should be permitted as a dotfile (rc=$rc)" "$out"
fi

# enforce + activation + explicit FLOX_SANDBOX_ALLOW grant of the sensitive
# path: the explicit grant wins, so the read is permitted silently. This proves
# the sensitive check runs AFTER the explicit allow checks.
out="$(run_activation enforce "$sens_home" \
    FLOX_SANDBOX_ALLOW="$sens_home/.ssh/**" -- open "$sens_home/.ssh/known_hosts" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" != *"sensitive"* ]]; then
  pass "enforce+activation: explicit allow grant overrides the sensitive set"
else
  fail "enforce+activation: an explicit FLOX_SANDBOX_ALLOW grant should win (rc=$rc)" "$out"
fi

# enforce + BUILD mode (no activation flag): the same sensitive path is NOT
# denied as sensitive — the sensitive set is activation-only, so build
# behaviour is unchanged (the dotfile carve-out permits it). This is the
# byte-identical-build guard for DX-2.
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    HOME="$sens_home" "$root/tests/sandbox_probe" open "$sens_home/.ssh/known_hosts" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" != *"sensitive"* ]]; then
  pass "enforce+build: sensitive set NOT applied (build behaviour unchanged)"
else
  fail "enforce+build: ~/.ssh should follow the build dotfile carve-out (rc=$rc)" "$out"
fi

rm -rf "$sens_home"

# Write-create guard. A create under an IN-POLICY dir (the fixture, an
# allow-dir) succeeds; a create under an OUT-OF-POLICY dir (a fresh temp dir
# that is not an allow-dir) is denied. The target file must not exist so the
# create path (realpath fails) is exercised.
in_policy_new="$fixture/created-by-probe-$$"
out="$(run_activation enforce "$HOME" -- create "$in_policy_new" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"CREATE_OK"* ]]; then
  pass "enforce+activation: create under an in-policy dir succeeds"
else
  fail "enforce+activation: create under the fixture (allow-dir) should succeed (rc=$rc)" "$out"
fi
rm -f "$in_policy_new"

# A create whose immediate parent is a freshly-made subtree UNDER an in-policy
# dir succeeds. This mirrors real git: it mkdir()s `.git/objects/<fanout>/`
# (mkdir is not intercepted) and then open(O_CREAT)s the temp object inside it,
# so at the open the immediate parent exists but is a directory the engine has
# never seen. The guard resolves that parent and finds it under the in-policy
# fixture. (We mkdir the subtree first because open(O_CREAT) itself never
# creates intermediate directories — only the leaf file.)
in_policy_subtree="$fixture/new/deep/subtree"
mkdir -p "$in_policy_subtree"
out="$(run_activation enforce "$HOME" -- create "$in_policy_subtree/created-by-probe-$$" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"CREATE_OK"* ]]; then
  pass "enforce+activation: create in a fresh subtree under an in-policy dir succeeds"
else
  fail "enforce+activation: subtree create under the fixture should succeed (rc=$rc)" "$out"
fi
rm -rf "$fixture/new"

# The walk-up itself: a create whose immediate parent does NOT yet exist but
# whose nearest existing ancestor IS in policy is permitted by the sandbox. The
# kernel still returns ENOENT (open does not create intermediate dirs), but the
# point is the sandbox must NOT have denied it — assert no SANDBOX ERROR line
# even though the open ultimately fails for the benign missing-dir reason.
out="$(run_activation enforce "$HOME" -- create "$fixture/notyet/leaf" 2>&1)"; rc=$?
if [[ "$out" != *"SANDBOX ERROR"* && "$out" == *"errno=2"* ]]; then
  pass "enforce+activation: create under an in-policy missing parent not denied by sandbox"
else
  fail "enforce+activation: walk-up should permit an in-policy create (sandbox must not deny) (rc=$rc)" "$out"
fi

# An out-of-policy create dir: a temp dir under the real $HOME that is neither
# an allow-dir, the closure, nor the store. The create is denied (fatal under
# enforce) by the parent-dir policy check.
oop_dir="$(mktemp -d "$HOME/flox-sandbox-tests-oop.XXXXXX")"
out="$(run_activation enforce "$HOME" -- create "$oop_dir/pwned" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"is not in the sandbox"* ]]; then
  pass "enforce+activation: create under an out-of-policy dir is denied"
else
  fail "enforce+activation: out-of-policy new-file create should be denied (rc=$rc)" "$out"
fi

# A create whose nearest existing ancestor is OUT of policy is still denied,
# even when the immediate parent does not exist: walking up to $oop_dir finds it
# out of policy. This proves the walk-up does not weaken the threat model.
out="$(run_activation enforce "$HOME" -- create "$oop_dir/new/deep/pwned" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"is not in the sandbox"* ]]; then
  pass "enforce+activation: deep create under an out-of-policy dir is denied"
else
  fail "enforce+activation: out-of-policy deep create should be denied (rc=$rc)" "$out"
fi

# Build mode (no activation flag): BOTH creates are allowed — the engine keeps
# its blanket-allow of nonexistent paths for builds, which legitimately create
# many new files. This is the byte-identical-build guard for DX-3.
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    HOME="$HOME" "$root/tests/sandbox_probe" create "$oop_dir/pwned" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"CREATE_OK"* ]]; then
  pass "enforce+build: out-of-policy create allowed (build behaviour unchanged)"
else
  fail "enforce+build: build mode should permit any new-file create (rc=$rc)" "$out"
fi
rm -rf "$oop_dir"

# /nix/store read under enforce + a FLOX_SANDBOX_ALLOW_DIRS=/nix/store entry is
# permitted silently. The activation seed adds /nix/store to the allow-dirs; the
# equivalent engine-level assertion is that a store path under an allow-dir is
# allowed without a warning. out_file is a real store file.
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="/nix/store" FLOX_VIRTUAL_SANDBOX=enforce \
    FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1 \
    "$root/tests/sandbox_probe" open "$out_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" != *"not in the sandbox"* ]]; then
  pass "enforce+activation: /nix/store read permitted via allow-dir (DX-1)"
else
  fail "enforce+activation: a /nix/store read should be permitted silently (rc=$rc)" "$out"
fi

# ----------------------------------------------------------------------------
# Layer 4.7: write-path interposition (full write entry-point coverage +
# graceful denial).
#
# Two fixes are exercised here:
#   1. Write coverage. Beyond open()/openat(), the destructive write entry
#      points creat(), truncate(), and freopen() (plus the platform large-file
#      and $NOCANCEL variants) are interposed, so a write cannot slip past the
#      sandbox by binding a symbol the open interceptor never saw. A shell `>>`
#      redirect to a sensitive file is the canonical real-world case.
#   2. Graceful denial. A policy denial is reported and refused with EACCES in
#      EVERY mode — never exit(1). A shell builtin redirect performs its open
#      inside the interactive shell process, so a fatal denial would kill the
#      user's shell. enforce/pure now refuse with a clean error like prompt.
#
# Unless noted, cases run under an activation (FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1).
# ----------------------------------------------------------------------------

wp_home="$(mktemp -d "$HOME/flox-sandbox-tests-home.XXXXXX")"
mkdir -p "$wp_home/.ssh"
printf 'known' > "$wp_home/.ssh/known_hosts"

# append to an EXISTING sensitive file is denied GRACEFULLY (EACCES), not fatal,
# and leaves the file untouched. Mirrors a shell `>>` to a credential file.
out="$(run_activation enforce "$wp_home" -- append "$wp_home/.ssh/known_hosts" 2>&1)"; rc=$?
contents="$(cat "$wp_home/.ssh/known_hosts")"
if [[ $rc -ne 0 && "$out" == *"APPEND_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"is not in the sandbox (sensitive)"* && "$contents" == "known" ]]; then
  pass "enforce+activation: append to an existing sensitive file denied (EACCES), file intact"
else
  fail "enforce+activation: sensitive append should be EACCES-denied, file unchanged (rc=$rc, contents=$contents)" "$out"
fi

# A denied write does NOT terminate a long-lived process: the probe attempts the
# denied append, then — still running — reads an in-policy file and prints
# SURVIVED. Under the old fatal model the process died at the append.
out="$(run_activation enforce "$wp_home" -- survive "$wp_home/.ssh/known_hosts" "$in_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"APPEND_DENIED"* && "$out" == *"errno=13"* \
      && "$out" == *"READ_OK"* && "$out" == *"SURVIVED"* ]]; then
  pass "enforce+activation: a denied write does not kill the process (graceful EACCES)"
else
  fail "enforce+activation: process should survive a denied write (rc=$rc)" "$out"
fi

rm -rf "$wp_home"

# New-file create of a SENSITIVE path under an IN-POLICY directory is denied:
# the parent ($fixture, an allow-dir) is in policy, but the target itself is
# sensitive (**/.env), so the create is refused — consistent with the
# existing-file sensitive denial (closes the create-branch sensitive gap).
out="$(run_activation enforce "$HOME" -- create "$fixture/.env" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"is not in the sandbox (sensitive)"* ]]; then
  pass "enforce+activation: new sensitive file create under an in-policy dir denied"
else
  fail "enforce+activation: creating \$fixture/.env should be denied as sensitive (rc=$rc)" "$out"
fi
rm -f "$fixture/.env"

# The same create driven by a RELATIVE path: `cd $fixture` then create ".env".
# This is the natural agent invocation (`cd project && echo x > .env`), and the
# bare name matches neither `**/.env` (the pattern's literal '/' has no
# counterpart in the string under fnmatch flags=0) nor any `~/`-expanded
# absolute pattern — so without cwd-absolutization the create is judged only by
# the (in-policy) parent and slips through. Assert the denial AND that the
# report names the absolutized target (".../.env"), proving the engine matched
# the cwd-joined candidate, not the raw relative name.
out="$( cd "$fixture" && run_activation enforce "$HOME" -- create ".env" 2>&1 )"; rc=$?
if [[ $rc -ne 0 && "$out" == *"/.env is not in the sandbox (sensitive)"* ]]; then
  pass "enforce+activation: relative new sensitive file create denied (cwd-absolutized)"
else
  fail "enforce+activation: creating '.env' relative to an in-policy cwd should be denied as sensitive (rc=$rc)" "$out"
fi
rm -f "$fixture/.env"

# creat(): a new file under an OUT-OF-POLICY dir is denied. creat is a distinct
# entry point from open(O_CREAT); without its own interceptor the write would
# slip past the sandbox.
wp_oop="$(mktemp -d "$HOME/flox-sandbox-tests-oop.XXXXXX")"
out="$(run_activation enforce "$HOME" -- creat "$wp_oop/pwned" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"CREAT_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"is not in the sandbox"* ]]; then
  pass "enforce+activation: creat() under an out-of-policy dir is denied"
else
  fail "enforce+activation: creat() out-of-policy should be denied (rc=$rc)" "$out"
fi

# truncate(): truncating an EXISTING out-of-policy file is a destructive write
# and must be denied, leaving the file unchanged.
printf 'data' > "$wp_oop/victim"
out="$(run_activation enforce "$HOME" -- truncate "$wp_oop/victim" 2>&1)"; rc=$?
contents="$(cat "$wp_oop/victim")"
if [[ $rc -ne 0 && "$out" == *"TRUNCATE_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"is not in the sandbox"* && "$contents" == "data" ]]; then
  pass "enforce+activation: truncate() of an out-of-policy file is denied, file intact"
else
  fail "enforce+activation: truncate() out-of-policy should be denied, file unchanged (rc=$rc, contents=$contents)" "$out"
fi

# freopen(): reopening an EXISTING out-of-policy file for writing must be denied
# (freopen opens and truncates). A third distinct write entry point.
printf 'data' > "$wp_oop/freopen-victim"
out="$(run_activation enforce "$HOME" -- freopen "$wp_oop/freopen-victim" 2>&1)"; rc=$?
contents="$(cat "$wp_oop/freopen-victim")"
if [[ $rc -ne 0 && "$out" == *"FREOPEN_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"is not in the sandbox"* && "$contents" == "data" ]]; then
  pass "enforce+activation: freopen() of an out-of-policy file is denied, file intact"
else
  fail "enforce+activation: freopen() out-of-policy should be denied, file unchanged (rc=$rc, contents=$contents)" "$out"
fi
rm -rf "$wp_oop"

# In-policy writes still pass on every new entry point. The fixture is an
# allow-dir ($TMPDIR), so appending/creating/truncating there is permitted
# silently — the write interception must not regress legitimate in-project edits.
printf 'x' > "$fixture/wp-existing"
out="$(run_activation enforce "$HOME" -- append "$fixture/wp-existing" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"APPEND_OK"* && "$out" != *"not in the sandbox"* ]]; then
  pass "enforce+activation: append to an in-policy file still permitted"
else
  fail "enforce+activation: in-policy append should succeed (rc=$rc)" "$out"
fi
out="$(run_activation enforce "$HOME" -- creat "$fixture/wp-created" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"CREAT_OK"* && "$out" != *"not in the sandbox"* ]]; then
  pass "enforce+activation: creat() in an in-policy dir still permitted"
else
  fail "enforce+activation: in-policy creat should succeed (rc=$rc)" "$out"
fi
out="$(run_activation enforce "$HOME" -- truncate "$fixture/wp-existing" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"TRUNCATE_OK"* && "$out" != *"not in the sandbox"* ]]; then
  pass "enforce+activation: truncate() of an in-policy file still permitted"
else
  fail "enforce+activation: in-policy truncate should succeed (rc=$rc)" "$out"
fi
rm -f "$fixture/wp-existing" "$fixture/wp-created"

# $TMPDIR write: a built-in allow-dir, so a create directly under it succeeds.
tmpdir_target="${TMPDIR:-/tmp}/flox-sandbox-wp-$$"
out="$(run_activation enforce "$HOME" -- create "$tmpdir_target" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"CREATE_OK"* && "$out" != *"not in the sandbox"* ]]; then
  pass "enforce+activation: \$TMPDIR write permitted"
else
  fail "enforce+activation: a \$TMPDIR write should succeed (rc=$rc)" "$out"
fi
rm -f "$tmpdir_target"

# Byte-identical build behaviour: WITHOUT the activation flag (build mode), the
# new write interceptors keep the historical allow-of-nonexistent — a build
# legitimately creates many new files. creat() under an out-of-policy dir is
# permitted in build mode, unchanged.
wp_build_oop="$(mktemp -d "$HOME/flox-sandbox-tests-oop.XXXXXX")"
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    HOME="$HOME" "$root/tests/sandbox_probe" creat "$wp_build_oop/newfile" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"CREAT_OK"* ]]; then
  pass "enforce+build: creat() of a new file allowed (build behaviour unchanged)"
else
  fail "enforce+build: build mode should permit any new-file creat (rc=$rc)" "$out"
fi
rm -rf "$wp_build_oop"

# Real-tool coverage: an actual shell `>>` redirect to an existing sensitive
# file. The redirect binds a libc open variant (open$NOCANCEL on macOS, open64
# on glibc) that the plain interceptor historically could miss; the destination
# .env is sensitive. Assert the sandbox blocked it, the file is byte-for-byte
# unchanged, and the shell SURVIVED the denial (a later command ran) rather than
# being killed. Skipped when bash is unavailable or SIP-protected (macOS strips
# DYLD_INSERT_LIBRARIES for /bin, /usr/bin, ... binaries).
bash_bin="$(command -v bash 2>/dev/null || true)"
sip_bash=0
case "$bash_bin" in
  /bin/*|/usr/bin/*|/sbin/*|/usr/sbin/*) sip_bash=1 ;;
esac
if [[ -n "$bash_bin" && ( "$(uname -s)" != "Darwin" || $sip_bash -eq 0 ) ]]; then
  rt_home="$(mktemp -d "$HOME/flox-sandbox-tests-home.XXXXXX")"
  printf 'SECRET' > "$rt_home/.env"
  out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
      FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs /nix/store" FLOX_VIRTUAL_SANDBOX=enforce \
      FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1 HOME="$rt_home" \
      "$bash_bin" -c 'echo PWNED >> "$HOME/.env"; echo STILL_ALIVE' 2>&1)"; rc=$?
  contents="$(cat "$rt_home/.env")"
  rm -rf "$rt_home"
  if [[ "$out" == *"is not in the sandbox (sensitive)"* \
        && "$contents" == "SECRET" && "$out" == *"STILL_ALIVE"* ]]; then
    pass "enforce+activation: real shell '>>' to a sensitive file blocked, file intact, shell survives"
  else
    fail "enforce+activation: shell redirect should be denied gracefully, file unchanged (rc=$rc, contents=$contents)" "$out"
  fi
else
  echo "skip - bash unavailable or SIP-protected; real-tool '>>' write coverage needs a mediatable shell"
fi

# ----------------------------------------------------------------------------
# Layer 4.8: directory enumeration (opendir/fdopendir interposition).
#
# `ls` enumerates with opendir()/readdir(), which never routes through the
# interposed open()/openat() symbols — so before these interceptors a
# directory listing reached NO check at all: warn mode was silent and prompt
# listed unapproved directories without a receipt. Under an ACTIVATION
# (FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1) an out-of-policy enumeration is now
# mediated as a READ of the directory path with the unified severity model:
#   warn    -> report once, permit
#   enforce -> report once, graceful EACCES (opendir returns NULL; never fatal)
#   prompt  -> deny + queue receipt (fail-closed without a broker)
# In-policy directories (allow-dirs, the closure, seeded prefixes) stay
# enumerable and silent, and BUILD behaviour (no activation flag) keeps the
# historical warn-but-permit.
# ----------------------------------------------------------------------------

dir_home="$(mktemp -d "$HOME/flox-sandbox-tests-home.XXXXXX")"
mkdir -p "$dir_home/private-dir"
printf 'x' > "$dir_home/private-dir/file.txt"
oop_list_dir="$dir_home/private-dir"

# warn + activation: an out-of-policy listing is permitted but REPORTED (a
# silent `ls ~` was the bug). The message goes through the shared verdict
# helper, so it is the standard "not in the sandbox" line with the
# directory-listing qualifier.
out="$(run_activation warn "$dir_home" -- opendir "$oop_list_dir" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPENDIR_OK"* \
      && "$out" == *"is not in the sandbox (directory listing)"* ]]; then
  pass "warn+activation: out-of-policy opendir permitted but warned"
else
  fail "warn+activation: opendir should warn-and-permit (rc=$rc)" "$out"
fi

# enforce + activation: the same listing is refused with a GRACEFUL EACCES —
# opendir returns NULL, the probe prints OPENDIR_FAIL and exits 1 cleanly
# (never killed), and no entry name was enumerable.
out="$(run_activation enforce "$dir_home" -- opendir "$oop_list_dir" 2>&1)"; rc=$?
if [[ $rc -eq 1 && "$out" == *"OPENDIR_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"is not in the sandbox (directory listing)"* ]]; then
  pass "enforce+activation: out-of-policy opendir denied with graceful EACCES"
else
  fail "enforce+activation: opendir should EACCES, not crash (rc=$rc)" "$out"
fi

# prompt + activation, no broker socket: fail-closed deny — EACCES plus the
# distinct fail-closed receipt, exactly as for any other prompt-denied read.
out="$(run_activation prompt "$dir_home" -- opendir "$oop_list_dir" 2>&1)"; rc=$?
if [[ $rc -eq 1 && "$out" == *"OPENDIR_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"denying read of"*"(fail-closed"* ]]; then
  pass "prompt+activation: out-of-policy opendir fail-closed EACCES without broker"
else
  fail "prompt+activation: opendir should fail closed without a broker (rc=$rc)" "$out"
fi

# prompt + activation + live broker deny: EACCES plus the two-line deny+queue
# receipt (enumeration without a receipt was the bug).
if command -v python3 >/dev/null 2>&1 && start_fake_broker deny; then
  out="$(run_activation prompt "$dir_home" FLOX_SANDBOX_PROMPT_SOCKET="$broker_sock" \
      -- opendir "$oop_list_dir" 2>&1)"; rc=$?
  stop_fake_broker
  if [[ $rc -eq 1 && "$out" == *"OPENDIR_FAIL"* && "$out" == *"errno=13"* \
        && "$out" == *"read $oop_list_dir (not in policy)"* \
        && "$out" == *"queued as req"* ]]; then
    pass "prompt+activation: out-of-policy opendir denied + queued receipt"
  else
    fail "prompt+activation: opendir deny should EACCES + queue (rc=$rc)" "$out"
  fi
else
  stop_fake_broker
  pass "prompt broker opendir test skipped (python3 unavailable)"
fi

# In-policy directories stay enumerable and SILENT in every mode: the fixture
# is under an allow-dir, so the listing must succeed without a single sandbox
# line (under prompt this also proves no RPC is attempted — a fail-closed
# receipt would appear, since no socket is configured).
for mode in warn enforce prompt; do
  out="$(run_activation "$mode" "$dir_home" -- opendir "$fixture" 2>&1)"; rc=$?
  if [[ $rc -eq 0 && "$out" == *"OPENDIR_OK"* && "$out" != *"SANDBOX"* ]]; then
    pass "$mode+activation: in-policy opendir permitted silently"
  else
    fail "$mode+activation: in-policy opendir should be silent (rc=$rc)" "$out"
  fi
done

# Seeded-profile prefix: the activation seed adds /nix/store as an allow-dir;
# a store directory must stay enumerable and silent under enforce (the
# engine-level equivalent of "the seeded profile keeps the shell quiet").
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs /nix/store" FLOX_VIRTUAL_SANDBOX=enforce \
    FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1 HOME="$dir_home" \
    "$root/tests/sandbox_probe" opendir "$out_store" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPENDIR_OK"* && "$out" != *"SANDBOX"* ]]; then
  pass "enforce+activation: seeded /nix/store dir opendir permitted silently"
else
  fail "enforce+activation: store dir should be enumerable via seeded allow-dir (rc=$rc)" "$out"
fi

# fdopendir(): the openat()+fdopendir() traversal style (find, fts). The
# open(O_DIRECTORY) itself is a warned-but-permitted probe, so the caller gets
# an fd even out of policy — fdopendir() maps the fd back to its directory
# path and applies the same directory-read verdict before any entry is
# readable. (The denial message is deduped against the probe warning for the
# same path, so only the EACCES is asserted here.)
out="$(run_activation enforce "$dir_home" -- fdopendir "$oop_list_dir" 2>&1)"; rc=$?
if [[ $rc -eq 1 && "$out" == *"FDOPENDIR_FAIL"* && "$out" == *"errno=13"* ]]; then
  pass "enforce+activation: out-of-policy fdopendir denied with graceful EACCES"
else
  fail "enforce+activation: fdopendir should EACCES on an out-of-policy dir (rc=$rc)" "$out"
fi
out="$(run_activation enforce "$dir_home" -- fdopendir "$fixture" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"FDOPENDIR_OK"* && "$out" != *"SANDBOX"* ]]; then
  pass "enforce+activation: in-policy fdopendir permitted silently"
else
  fail "enforce+activation: in-policy fdopendir should succeed silently (rc=$rc)" "$out"
fi

# BUILD mode (no activation flag): opendir of an out-of-closure directory
# keeps the historical warn-but-permit — the new denial is activation-only,
# so build behaviour stays byte-identical (mirrors the open()-based directory
# test in Layer 2).
out="$(run_probe enforce opendir "$out_store" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPENDIR_OK"* && "$out" == *"directory listing"* ]]; then
  pass "enforce+build: out-of-closure opendir still warn-but-permit"
else
  fail "enforce+build: build-mode opendir should warn and permit (rc=$rc)" "$out"
fi

# Shell startup is unaffected: a real shell launches, runs a command, and
# exits cleanly under enforce+activation with the seeded-style allow set —
# directory mediation must not break the interactive entry path. Skipped when
# bash is SIP-protected (macOS strips DYLD_INSERT_LIBRARIES for /bin/...).
if [[ -n "$bash_bin" && ( "$(uname -s)" != "Darwin" || $sip_bash -eq 0 ) ]]; then
  out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
      FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs /nix/store" FLOX_VIRTUAL_SANDBOX=enforce \
      FLOX_SANDBOX_ALLOW_FOREIGN_EXE=1 HOME="$dir_home" \
      "$bash_bin" -c 'echo SHELL_OK' 2>&1)"; rc=$?
  if [[ $rc -eq 0 && "$out" == *"SHELL_OK"* && "$out" != *"SANDBOX ERROR"* ]]; then
    pass "enforce+activation: shell startup unaffected by directory mediation"
  else
    fail "enforce+activation: shell should start and run under enforce (rc=$rc)" "$out"
  fi
else
  echo "skip - bash unavailable or SIP-protected; shell-startup coverage needs a mediatable shell"
fi

rm -rf "$dir_home"

# ----------------------------------------------------------------------------
# Layer 5: network egress (connect interception, warn/enforce gradient).
#
# These use only loopback and TEST-NET-1 (192.0.2.0/24, RFC 5737, guaranteed
# unroutable) so there is no real internet dependency and nothing can hang: the
# probe uses a non-blocking socket with a short poll timeout, and under enforce
# the sandbox refuses BEFORE the syscall so there is no network wait at all.
#
# The "network policy" substring is the sandbox's own message (warn emits
# SANDBOX WARNING ... "is not in the network policy"; enforce emits SANDBOX
# ERROR with the same tail). Its presence/absence is the discriminator between
# "sandbox mediated this" and "sandbox stayed out of it".
# ----------------------------------------------------------------------------

# Loopback is always allowed, silently, in every mode. Connecting to a
# loopback port with nothing listening yields a kernel ECONNREFUSED, but the
# SANDBOX must NOT have mediated it: assert no "network policy" line appears.
for mode in off warn enforce; do
  out="$(run_probe "$mode" connect 127.0.0.1 1 2>&1)"; rc=$?
  if [[ "$out" != *"network policy"* ]]; then
    pass "$mode: loopback connect not mediated by sandbox (silent)"
  else
    fail "$mode: loopback connect should never hit the network policy" "$out"
  fi
done

# warn: a connect to an out-of-policy NON-loopback dest is reported but
# PERMITTED — the sandbox does not force ECONNREFUSED. 192.0.2.1 is unroutable,
# so the non-blocking connect proceeds (EINPROGRESS) and times out at the
# network layer; the probe prints CONNECT_PROCEEDED (exit 0). The warning line
# must be present, and the probe must NOT report CONNECT_REFUSED.
out="$(run_probe warn connect 192.0.2.1 443 2>&1)"; rc=$?
if [[ "$out" == *"connect to 192.0.2.1:443 is not in the network policy"* \
      && "$out" != *"CONNECT_REFUSED"* && "$out" == *"CONNECT_PROCEEDED"* ]]; then
  pass "warn: out-of-policy connect warned but permitted (no sandbox refusal)"
else
  fail "warn: expected WARNING + CONNECT_PROCEEDED, not refused (rc=$rc)" "$out"
fi

# enforce: the same out-of-policy connect is refused by the sandbox
# IMMEDIATELY with ECONNREFUSED — before the real syscall, so there is no
# network wait (the probe returns at once with CONNECT_REFUSED, not a timeout).
out="$(run_probe enforce connect 192.0.2.1 443 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"CONNECT_REFUSED"* && "$out" == *"errno=$ECONNREFUSED_NO"* \
      && "$out" == *"is not in the network policy"* ]]; then
  pass "enforce: out-of-policy connect refused with ECONNREFUSED + ERROR line"
else
  fail "enforce: out-of-policy connect should be refused immediately (rc=$rc)" "$out"
fi

# enforce + FLOX_SANDBOX_ALLOW_NET lists the exact IP → the connect is
# permitted silently (no "network policy" line, no sandbox refusal). It still
# fails/times out at the network layer because 192.0.2.1 is unroutable, but
# that is the kernel, not the sandbox: assert CONNECT_PROCEEDED and no policy
# message.
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    FLOX_SANDBOX_ALLOW_NET="192.0.2.1" \
    "$root/tests/sandbox_probe" connect 192.0.2.1 443 2>&1)"; rc=$?
if [[ "$out" != *"network policy"* && "$out" != *"CONNECT_REFUSED"* ]]; then
  pass "enforce: FLOX_SANDBOX_ALLOW_NET exact-IP entry permits the connect"
else
  fail "enforce: allow-net IP should silently permit the connect (rc=$rc)" "$out"
fi

# enforce + a CIDR allow-net entry covering the dest → permitted silently.
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    FLOX_SANDBOX_ALLOW_NET="192.0.2.0/24" \
    "$root/tests/sandbox_probe" connect 192.0.2.1 443 2>&1)"; rc=$?
if [[ "$out" != *"network policy"* && "$out" != *"CONNECT_REFUSED"* ]]; then
  pass "enforce: FLOX_SANDBOX_ALLOW_NET CIDR entry permits the connect"
else
  fail "enforce: allow-net CIDR should silently permit the connect (rc=$rc)" "$out"
fi

# enforce + a port-qualified allow-net entry only matches that port: the dest
# port (443) differs from the listed port (80), so the connect is still
# refused.
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    FLOX_SANDBOX_ALLOW_NET="192.0.2.1:80" \
    "$root/tests/sandbox_probe" connect 192.0.2.1 443 2>&1)"; rc=$?
if [[ "$out" == *"CONNECT_REFUSED"* && "$out" == *"is not in the network policy"* ]]; then
  pass "enforce: port-qualified allow-net entry does not match a different port"
else
  fail "enforce: 192.0.2.1:80 must not permit a :443 connect (rc=$rc)" "$out"
fi

# prompt has no network broker yet, so it applies enforce semantics for the
# network: an out-of-policy connect is refused with ECONNREFUSED (the
# filesystem prompt flow is unaffected). This guards the documented interim
# decision.
out="$(run_probe_activation prompt connect 192.0.2.1 443 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"CONNECT_REFUSED"* \
      && "$out" == *"is not in the network policy"* ]]; then
  pass "prompt: out-of-policy connect refused (enforce semantics, no net broker yet)"
else
  fail "prompt: out-of-policy connect should be refused under ask (rc=$rc)" "$out"
fi

# ----------------------------------------------------------------------------
# Layer 6: audit store (audit.ndjson appended by the engine).
#
# Every report the engine emits — warn-mode reports and enforce/prompt denials,
# for files, directory enumerations, and network connects — must also land as
# one NDJSON record in $FLOX_SANDBOX_GRANTS_DIR/audit.ndjson so it is
# queryable after the session (`flox sandbox audit`). The hook rides the
# once-per-key dedup, so repeated accesses append exactly one record. Without
# FLOX_SANDBOX_GRANTS_DIR (every build, and all the layers above) the hook is
# inert and no file is created.
# ----------------------------------------------------------------------------

audit_home="$(mktemp -d "$HOME/flox-sandbox-tests-home.XXXXXX")"
audit_dir="$(mktemp -d "${TMPDIR:-/tmp}/flox-sandbox-tests-audit.XXXXXX")"
audit_file="$audit_dir/audit.ndjson"
printf 'secret' > "$audit_home/outside.txt"

# Helper: count audit records matching a substring.
audit_count() {
  [[ -f "$audit_file" ]] || { echo 0; return; }
  grep -c "$1" "$audit_file" || true
}

# warn + activation: an out-of-policy read appends exactly ONE record per
# path (the storm touches it 8 times), with mode/kind/op/verdict fields.
rm -f "$audit_file"
out="$(run_activation warn "$audit_home" FLOX_SANDBOX_GRANTS_DIR="$audit_dir" \
      -- storm 1 8 "$audit_home/outside.txt" 2>&1)"; rc=$?
records="$(audit_count "$audit_home/outside.txt")"
if [[ -f "$audit_file" && "$records" -eq 1 \
      && "$(audit_count '"mode":"warn"')" -eq 1 \
      && "$(audit_count '"kind":"fs"')" -eq 1 \
      && "$(audit_count '"op":"read"')" -eq 1 \
      && "$(audit_count '"verdict":"warned"')" -eq 1 ]]; then
  pass "audit: warn-mode report appends exactly one fs record per path"
else
  fail "audit: warn report should append one record (rc=$rc, records=$records)" \
       "$(cat "$audit_file" 2>/dev/null)"
fi

# enforce: a denial appends a record with verdict "denied"; the pid and exe
# attribution fields are populated (exe ends in sandbox_probe).
rm -f "$audit_file"
out="$(run_activation enforce "$audit_home" FLOX_SANDBOX_GRANTS_DIR="$audit_dir" \
      -- open "$audit_home/outside.txt" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$(audit_count '"verdict":"denied"')" -eq 1 \
      && "$(audit_count '"mode":"enforce"')" -eq 1 \
      && "$(audit_count 'sandbox_probe')" -ge 1 \
      && "$(audit_count '"pid":')" -eq 1 ]]; then
  pass "audit: enforce denial recorded with exe/pid attribution"
else
  fail "audit: enforce denial should append a denied record (rc=$rc)" \
       "$(cat "$audit_file" 2>/dev/null)"
fi

# enforce: a WRITE denial records op "write" (the new write entry points
# route through the same audited verdict tail).
rm -f "$audit_file"
out="$(run_activation enforce "$audit_home" FLOX_SANDBOX_GRANTS_DIR="$audit_dir" \
      -- append "$audit_home/outside.txt" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$(audit_count '"op":"write"')" -eq 1 \
      && "$(audit_count '"verdict":"denied"')" -eq 1 ]]; then
  pass "audit: enforce write denial recorded with op write"
else
  fail "audit: write denial should append an op:write record (rc=$rc)" \
       "$(cat "$audit_file" 2>/dev/null)"
fi

# enforce: a directory ENUMERATION denial is recorded as a read of the dir
# path (the dir verdict routes through the shared audited tail).
rm -f "$audit_file"
mkdir -p "$audit_home/private"
printf 'x' > "$audit_home/private/file"
out="$(run_activation enforce "$audit_home" FLOX_SANDBOX_GRANTS_DIR="$audit_dir" \
      -- opendir "$audit_home/private" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$(audit_count "$audit_home/private")" -ge 1 \
      && "$(audit_count '"op":"read"')" -eq 1 \
      && "$(audit_count '"verdict":"denied"')" -eq 1 ]]; then
  pass "audit: directory-enumeration denial recorded as a read of the dir"
else
  fail "audit: opendir denial should append a record for the dir (rc=$rc)" \
       "$(cat "$audit_file" 2>/dev/null)"
fi

# prompt with no broker socket: the fail-closed denial is recorded with
# verdict "fail-closed" and mode "prompt".
rm -f "$audit_file"
out="$(run_activation prompt "$audit_home" FLOX_SANDBOX_GRANTS_DIR="$audit_dir" \
      -- open "$audit_home/outside.txt" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$(audit_count '"verdict":"fail-closed"')" -eq 1 \
      && "$(audit_count '"mode":"prompt"')" -eq 1 ]]; then
  pass "audit: prompt fail-closed denial recorded"
else
  fail "audit: broker-less prompt denial should record fail-closed (rc=$rc)" \
       "$(cat "$audit_file" 2>/dev/null)"
fi

# enforce: a refused connect appends a kind:net record with the destination
# and op connect.
rm -f "$audit_file"
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    FLOX_SANDBOX_GRANTS_DIR="$audit_dir" \
    "$root/tests/sandbox_probe" connect 192.0.2.1 443 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$(audit_count '"kind":"net"')" -eq 1 \
      && "$(audit_count '"op":"connect"')" -eq 1 \
      && "$(audit_count '192.0.2.1:443')" -eq 1 \
      && "$(audit_count '"verdict":"denied"')" -eq 1 ]]; then
  pass "audit: refused connect recorded as a net denial"
else
  fail "audit: net refusal should append a kind:net record (rc=$rc)" \
       "$(cat "$audit_file" 2>/dev/null)"
fi

# warn: an out-of-policy connect that is permitted-with-warning records
# verdict "warned".
rm -f "$audit_file"
out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=warn \
    FLOX_SANDBOX_GRANTS_DIR="$audit_dir" \
    "$root/tests/sandbox_probe" connect 192.0.2.1 443 2>&1)"; rc=$?
if [[ "$(audit_count '"kind":"net"')" -eq 1 \
      && "$(audit_count '"verdict":"warned"')" -eq 1 ]]; then
  pass "audit: warned connect recorded as a net warning"
else
  fail "audit: net warning should append a warned record (rc=$rc)" \
       "$(cat "$audit_file" 2>/dev/null)"
fi

# Without FLOX_SANDBOX_GRANTS_DIR no audit file is ever created — the hook is
# inert for builds (which never set the var). run_activation does not set it
# here, and the denial still happens on stderr.
rm -f "$audit_file"
out="$(run_activation enforce "$audit_home" -- open "$audit_home/outside.txt" 2>&1)"; rc=$?
if [[ $rc -ne 0 && ! -f "$audit_file" && "$out" == *"is not in the sandbox"* ]]; then
  pass "audit: no grants dir, no audit file (hook inert for builds)"
else
  fail "audit: without FLOX_SANDBOX_GRANTS_DIR nothing may be written (rc=$rc)" "$out"
fi

rm -rf "$audit_home" "$audit_dir"

# ----------------------------------------------------------------------------
# Summary.
# ----------------------------------------------------------------------------
echo
echo "# ${tests_run} tests, ${tests_failed} failed"
[[ $tests_failed -eq 0 ]]
