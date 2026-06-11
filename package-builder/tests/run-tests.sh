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
# Layer 4: ask mode (stub broker — deny-all out of policy via graceful EACCES).
#
# In this batch the broker RPC is not wired, so ask deterministically denies
# any out-of-policy access: sandbox_check_path() returns false, the
# interceptor's errno=EACCES branch fires, and the calling process sees a
# clean permission error (it is NOT aborted, unlike enforce). A two-line
# SANDBOX DENIED receipt is printed once per resolved path.
# ----------------------------------------------------------------------------

# ask + no socket env → deny. An out-of-closure read fails with EACCES (the
# probe sees open() return -1 and continues to print OPEN_FAIL — no crash, no
# exit(1) of the probe), and exactly one "SANDBOX DENIED ... not in policy"
# receipt appears. run_probe sets no FLOX_SANDBOX_SOCKET, so this is the
# unconfigured-broker case.
out="$(run_probe ask open "$out_file" 2>&1)"; rc=$?
denied_n="$(grep -c "SANDBOX DENIED" <<<"$out")"
if [[ $rc -ne 0 && "$out" == *"OPEN_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"read $out_file (not in policy)"* && "$denied_n" -eq 2 ]]; then
  pass "ask: out-of-closure read denied with graceful EACCES + receipt"
else
  fail "ask: expected EACCES (errno=13) + two-line DENIED receipt (rc=$rc, DENIED lines=$denied_n)" "$out"
fi

# Dotfile flip. Under ask the $HOME-dotfile carve-out is skipped, so a read of
# an out-of-closure $HOME dotfile is DENIED (EACCES + receipt). Under enforce
# the same read is PERMITTED with the existing "$HOME dotfile" warn line. The
# dotfile lives under a temp HOME that is not an allow-dir prefix, so the access
# genuinely exercises the home-dotfile branch rather than a directory allow.
home_dir="$(mktemp -d "$HOME/flox-sandbox-tests-home.XXXXXX")"
printf 'x' > "$home_dir/.fakerc"

out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=ask \
    HOME="$home_dir" "$root/tests/sandbox_probe" open "$home_dir/.fakerc" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"OPEN_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"read $home_dir/.fakerc (not in policy)"* ]]; then
  pass "ask: \$HOME dotfile read denied (dotfile carve-out skipped under ask)"
else
  fail "ask: \$HOME dotfile should be denied under ask (rc=$rc)" "$out"
fi

out="$(env "$preload_var=$sandbox_lib" FLOX_ENV="$fixture" \
    FLOX_SANDBOX_ALLOW_DIRS="$allow_dirs" FLOX_VIRTUAL_SANDBOX=enforce \
    HOME="$home_dir" "$root/tests/sandbox_probe" open "$home_dir/.fakerc" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* \
      && "$out" == *"permitted as a \$HOME dotfile"* ]]; then
  pass "enforce: same \$HOME dotfile still permitted (carve-out intact off-ask)"
else
  fail "enforce: \$HOME dotfile should remain permitted under enforce (rc=$rc)" "$out"
fi
rm -rf "$home_dir"

# Golden stability: a NORMAL out-of-closure file (not a dotfile) must behave
# exactly as before under warn and enforce — ask must not have perturbed the
# other levels. warn warns-but-permits; enforce is fatal with the same message.
out="$(run_probe warn open "$out_file" 2>&1)"; rc=$?
if [[ $rc -eq 0 && "$out" == *"OPEN_OK"* && "$out" == *"$out_file is not in the sandbox"* ]]; then
  pass "warn: normal out-of-closure read unchanged by ask addition"
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

# ask + flag: the inner shell no longer exit(1)s on the foreign exe. A foreign
# exe reading an OUT-OF-POLICY file gets the graceful ask deny (EACCES + receipt)
# — never the exe abort — so an activation completes past the shell.
out="$(run_foreign ask 1 open "$out_file" 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"OPEN_FAIL"* && "$out" == *"errno=13"* \
      && "$out" == *"read $out_file (not in policy)"* \
      && "$out" != *"process executable"* ]]; then
  pass "ask: flag exempts the foreign exe; out-of-policy read still EACCES-denied"
else
  fail "ask: foreign exe + flag should EACCES-deny the file, not abort on the exe (rc=$rc)" "$out"
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

# ask has no network broker yet, so it applies enforce semantics for the
# network: an out-of-policy connect is refused with ECONNREFUSED (the
# filesystem ask flow is unaffected). This guards the documented interim
# decision.
out="$(run_probe ask connect 192.0.2.1 443 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$out" == *"CONNECT_REFUSED"* \
      && "$out" == *"is not in the network policy"* ]]; then
  pass "ask: out-of-policy connect refused (enforce semantics, no net broker yet)"
else
  fail "ask: out-of-policy connect should be refused under ask (rc=$rc)" "$out"
fi

# ----------------------------------------------------------------------------
# Summary.
# ----------------------------------------------------------------------------
echo
echo "# ${tests_run} tests, ${tests_failed} failed"
[[ $tests_failed -eq 0 ]]
