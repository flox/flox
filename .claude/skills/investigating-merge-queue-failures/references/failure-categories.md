# Merge-queue failure categories

A catalog of *kinds* of failure and the signatures that identify them.
Deliberately no dates, run IDs, or occurrence counts — those belong in a
report, not here. Add a category when a signature matches nothing below.

Categories are organised by **what we would do about it**, not by mechanism,
because that is the decision the report has to support:

| | | |
|---|---|---|
| **1** | Not flaky at all | Fix the code; never retry |
| **2** | Flaky, not ours to fix | Retry, exclude from statistics |
| **3** | Flaky, ours to fix — builder instability | Fleet work |
| **4** | Flaky, ours to fix — tests written flakily | Test work |

Sections 3 and 4 are the ones worth counting, and the split between them is
what the report is for: it says whether the next block of effort should go into
the fleet or into the test suite. A single combined "flaky" number cannot
answer that, so it cannot change what anyone prioritises.

"Section 5: diagnosing a 30-minute timeout" is not a category — it is a routing
procedure, because that one symptom lands in three different categories above.

---

# 1. Not flaky — the queue doing its job

The queue tests something the PR's own CI never did, so these are real and
reproduce. **Fix the code. Retrying can never help.**

## 1a. Semantic conflict between PRs

**Signature:** the *same* test or build step fails identically on **every
platform in the run**, with a missing-symbol shape:

```
`<helper>' failed with status 127
<file>.bats: line <N>: <helper>: command not found
```

…or a compile error for a name that exists on neither side alone.

The queue builds the PR on top of everything ahead of it in the group, so two
PRs that each pass alone break when combined — typically one adds callers of a
helper another adds, or removes something a third still uses.

**Tell it apart from a flake:** **uniformity across platforms is the signal.**
A flake is platform-skewed — darwin but not linux, one arch but not the other.
This fails the same way everywhere at once.

**Repetition is not the signal.** Do not conclude "it failed twice on the same
head SHA, so it reproduces, so it is real." Both flaky categories repeat
readily: when infrastructure is bogged down the same PR fails the same
infrastructural way on consecutive attempts, and a well-established flaky test
can fail identically on two runs in a row. Observed semantic conflicts have
been caught on a *single* run.

## 1b. A check that is stricter in the queue than on the PR

**Signature:** `commit validation: failed!` in the `Nix Git Hooks` job, with:

```
commitizen-in-ci.........................................................Failed
- hook id: commitizen-in-ci
- exit code: 14
```

This is reachable *because the queue is deliberately stricter*.
`pkgs/pre-commit-check/default.nix` narrows commitizen's allowed commit
prefixes to `Revert` alone when `IS_MERGE_QUEUE=1`, where PR CI accepts
commitizen's defaults (`Merge`, `fixup!`, `squash!`, …). A branch carrying a
`fixup!` or `squash!` commit therefore passes its own CI by design and fails
the queue by design.

A `~/.netrc file owner ... does not match current user` warning often appears
alongside and is a red herring — it is a warning, not the failure.

**Action:** squash or reword the offending commits. When adding any check, note
whether it behaves differently under `IS_MERGE_QUEUE`; that difference is the
whole mechanism by which a queue-only, non-flaky failure exists.

---

# 2. Flaky, but not ours to fix

Third-party outages and queue bookkeeping. **Retry, and exclude these from
flake statistics** — counting them inflates the number without giving anyone
something to do. Identify by hostname: `api.github.com`, `github.com`, and
`releases.nixos.org` are not ours.

## 2a. Merge-queue ref race

Not instability at all — pure bookkeeping, filed here because the response is
the same: ignore it.

**Signature:** git exit code `128` with:

```
fatal: couldn't find remote ref refs/heads/gh-readonly-queue/main/pr-<N>-<sha>
```

The queue branch was deleted while the job was still checking it out — the
group was dequeued underneath a job that had already started. It tells you
nothing about the code and nothing about the fleet.

## 2b. GitHub API and GitHub-hosted runners

**Signature:** curl exit codes against `api.github.com` or `github.com`:

```
Could not resolve hostname (6)
Timeout was reached (28)
```

Also covers failures on GitHub's own runner images (the `macos-14-xlarge`
label and friends).

## 2c. Upstream Nix distribution

**Signature:** the Nix *installation* step fails or hangs, with curl verbose
output against `releases.nixos.org` and no completion. The job dies before any
test runs.

---

# 3. Flaky and ours to fix — builder instability

Our own builders, cache, and tailnet. **A recurring one is a fleet problem, not
bad luck.** This is where most infrastructure failures land.

Ours means: `s3://flox-cache-public` (our substituter), the tailnet, and any
machine drawn from `/etc/nix/machines` via `REMOTE_SERVER_ENTRY` — `hetzner-*`,
`indigo-*`, `lhr-*-darwin-*`, `flox-aarch64-darwin`.

## 3a. Slow builder — `timeout` expiry inside a test

**Signature:** exit status `124` anywhere in the log:

```
`wait_for_partial_file_content "$executive_log" "<text>"' failed with status 124
status : 124
```

`124` is what `timeout(1)` returns when it kills its child. Something did not
happen inside a hard-coded budget — which is *either* a slow builder or a wait
too tight to absorb normal variance.

**A 124 does not by itself mean the builder was slow. Measure before you
decide** — but measure the right thing.

**Do not use suite wall time.** It is contaminated: an expired wait *is* time
spent, so a job where twenty teardowns each burn ~19s on a bounded wait instead
of ~0.5s inflates its own wall clock by six minutes. Concluding "slow builder"
from that is circular — the timeouts caused the slowness you are citing as
their cause.

**Use the duration of the tests that passed.** Those are unaffected by whatever
timed out:

```
grep -aE '^[0-9T:.Z-]+ ok [0-9]+ .* # in [0-9]+ ms' <log> \
  | grep -oE '# in [0-9]+ ms' | grep -oE '[0-9]+' | sort -n
```

Take the median and the total, and compare two ways:

| Comparison | Why |
|---|---|
| Against a **healthy run of the same job** | The obvious baseline, but weak on its own — a reference from another week may differ for reasons unrelated to health. |
| Against the **other failed jobs on the same platform in the same period** | Much stronger. A builder problem makes one job an outlier among its peers; a systematically different baseline moves all of them together. |

A job that is ~1.5× a stale baseline *along with every one of its siblings* is
probably a baseline artefact. A job that is ~2× its own siblings, with several
times their number of failing tests, is a degraded machine — category **3**.
Everything else stays in category **4**.

Reference passing-test medians, for orientation only:

| Job | Tests | Median passing test |
|---|---|---|
| `remote (aarch64-darwin, !containerize)` | ~750 | ~505 ms |
| `remote (x86_64-darwin, !containerize,!activate)` | ~435 | ~1200 ms |

**Action when it is a tight wait:** make the wait poll a condition with a
generous ceiling rather than sleep a fixed budget. Do not "fix" the behaviour
being waited on — it was working.

## 3b. Slow builder — suite exhausts the 30-minute job budget

**Signature:** the bats timeout error with a **gap of seconds** between the
last log line and the error, and a last finished test well short of the plan.
Nothing hung — the suite was still making progress when the clock ran out. See
section 5.

Wall time is the right measure *here* — unlike in 3a — but only once you have
confirmed the log contains no `124` and no bounded-wait failures. If it does,
the suite inflated its own clock and you are back to 3a's method.

**A partially-completed suite means the builder is slow. Full stop.** The
budget is not marginal: a healthy `remote (aarch64-linux, !containerize)` run
gets through all ~750 tests in **under eight minutes**, leaving more than
twenty minutes of headroom. A run that fails to finish in thirty is several
times slower than normal, not a suite that has outgrown its budget. Don't
reach for "maybe the suite got too big" — check the healthy wall time and the
ratio will be obvious.

**Action:** treat as builder capacity. Worth recording which builder it landed
on, as with 3d.

## 3c. Tailscale setup failure

**Signature:**

```
##[warning]Tailscale up attempt <N> failed: Error: Timeout
##[error]Timeout
```

…usually with `logtail: dial "log.tailscale.com:443" failed` nearby. The job
never reaches the build. The tailnet is how jobs reach our remote builders, so
this is ours even though Tailscale is a vendor.

## 3d. Toolchain crash on a remote builder

**Signature:**

```
clang: error: linker command failed with exit code 139
error: linking with `cc` failed: exit status: 1
error: could not compile `<crate>`
```

`139` is `128 + 11` — the linker took SIGSEGV. Not a code error; the same
source builds fine on retry. Check the `remote-builders:` line to see which
machine it landed on; recurrence on one host is a fleet problem.

## 3e. `podman machine start` hangs on darwin

**Signature:** a `containerize` job on darwin whose log ends at:

```
Creating podman machine
Starting podman machine
```

…followed by tens of minutes of nothing and then the job timeout. No test
output at all, though the plan line was printed.

`setup_file` in `cli/tests/containerize.bats` calls
`create_and_start_podman_machine` on darwin. When healthy this takes on the
order of ten seconds; when it hangs it never returns. The suite never reached
test 1, so no test is implicated — this is infrastructure, not category 4.

---

# 4. Flaky and ours to fix — tests written flakily

Genuinely intermittent, strongly darwin-skewed. Before landing anything here,
rule out 1a (same failure on every platform). If the log contains a `124` or a
failing bounded wait, also run 3a's check on the passing-test durations: a slow
builder tripping a wait looks exactly like a flaky test, and "fixing" the test
would be fixing the wrong thing.

## 4a. Leaked background process holding FD 3

The one category here whose symptom is a timeout rather than an assertion.

**Signature:** plan is `1..N`, the last line is `ok N` for that same `N`, and
the gap before the timeout is tens of minutes. Every test ran and the *suite
finished*; the job then sat idle until the job timeout.

bats keeps file descriptor 3 open for its own output. If a test backgrounds a
process that inherits FD 3 and the test then fails without reaping it, the
leaked process holds FD 3 open and bats never sees it close — so the run hangs.
The hang may land right after the offending test or at the end of the whole
suite; both have been observed.

**The lead is the failing tests earlier in the same log**, not the timeout.

**Action:** find the test that backgrounds a process without closing FD 3 —
redirect its FD 3 (`3>&-` or `3>/dev/null`), or reap the child.

## 4b. Services lifecycle

Tests around starting, stopping, and reloading `process-compose` services:

- `services stop after auto-activated environment is deactivated`
- `start: shuts down process-compose started by imperative start`
- `config reload: ...: picks up environment modifications ...`

Frequently reached via a `124` on `wait_for_partial_file_content`. Check the
suite wall time against 3a's baselines before blaming the fleet: these
typically finish the whole suite in well under ten minutes, which means the
wait had no headroom rather than the builder being slow. The fix is the wait —
which lives in the test suite, so it stays category 4.

## 4c. Auto-activation and re-activation

Directory-transition behaviour, sensitive to timing and to leftover state:

- `re-allowing a denied mid-stack env re-inserts it in ancestor order`
- `bash: re-entering a project after leaving re-activates it`
- `'flox deactivate' suppresses re-activation until the directory is left`

## 4d. Activation hooks and shell integration

Per-shell (bash/fish/tcsh/zsh) activation behaviour, usually failing across
several shells at once when it goes:

- `activate runs hook only once in nested activation`
- `'flox activate' modifies the current shell (<shell>)`
- `<shell>: ...: attach sets vars from profile`

**Watch for failures in `teardown` rather than the test body:**

```
(from function `teardown' in test file activate.bats, line NN)
  `wait_for_activations "$PROJECT_DIR" || return 1' failed
```

This is a bounded wait like 3a's `124`, just a different helper, and it fails
the same way: the test body passed and cleanup did not finish inside its
budget. The giveaway is a run where many of these fail at once with nearly
identical durations (~19 s) against healthy times of a few hundred
milliseconds — that uniformity is the ceiling, not the behaviour.

## 4e. Containerize / podman

Container build-and-run tests, on both linux and darwin:

- `container can be run with 'podman run' with/without -i'`
- `container is written to runtime when '--runtime <runtime>' is passed`
- `cmd can run binary from activated environment`

Distinct from 3e, which is the VM failing to start before any of these run.

## 4f. Language end-to-end tests

`flox init` scaffolding for a language toolchain, e.g.
`'flox init' sets up a local working Go module environment`. These install real
packages, so rule out sections 2 and 3 before calling one flaky.

## 4g. Assertions that discard the cause

Not a failure mode so much as a test-authoring defect that makes every other
category impossible to diagnose:

```
test <name> has been running for over 60 seconds
thread '<name>' panicked at <file>:<line>: assertion failed: result.is_ok()
```

The `Err` is thrown away, so the log records only that something failed slowly.
A test like this **cannot be classified**. Do not guess a category for it from
unrelated warnings elsewhere in the log — record it as uncategorisable and fix
the assertion.

**Action:** assert on the error value (`unwrap()`, `expect()`, or
`assert!(matches!(...))` on the variant) so the next reader gets a cause
instead of a boolean.

**The trap this creates.** Wanting a cause, it is tempting to reach up the log
for the nearest scary-looking line and treat it as the explanation. Nix logs
substituter failures as `warning:` and then builds from source, so a burst of
`unable to download ... Timeout was reached (28)` against our own cache is
**not a failure at all** — it appears in green, collapsed steps of jobs that
went on to succeed. Check whether a candidate cause is a `warning:` or an
`error:` before attributing anything to it. Warnings are not causes, and a
category built on them is a category that does not exist.

---

# 5. Diagnosing a 30-minute timeout

**Signature:**

```
##[error]The action 'Run Bats Tests (./#flox-cli-tests)' has timed out after 30 minutes.
```

Not a category. This one symptom routes to three different categories, and the
routing is mechanical:

1. **Measure the gap** between the last log line and the `##[error]`.
   - **Seconds** → nothing hung; the suite ran out of budget. **3b.** Do not go
     looking for a hung test, there isn't one.
   - **Tens of minutes** → something genuinely hung. Continue.
2. **Compare the plan line `1..N` to the number of the last test that
   finished.**
   - Last number **equals** `N` → the suite finished and the job hung
     afterwards. **4a**, FD 3.
   - **No test output at all** → it hung in setup. On darwin `containerize`
     that is **3e**, `podman machine start`; read the last few lines to confirm
     which step stalled before assuming.

Skipping step 1 invents hangs that never happened, and sends someone hunting a
deadlock in a test that merely got cut off mid-run.

---

# Naming a test that never printed

bats prints a test's name only *after* it completes, so a test that hangs — or
that was mid-flight when the budget expired — is anonymous. You are left with
the *number* of the last test that finished and nothing else.

The number is the one bats prints in each result line — `ok 433 <name>` is test
number 433 — counting up to the total in the plan line, `1..N`. So a log ending
at `ok 433` tells you test 434 is the one that never reported, but not what
test 434 is called.

Get the name from a run where that test did finish:

1. Find a **successful** run of the **same job name** from around the same
   date (`gh run list --status success`, then match `.jobs[].name`).
2. Download its log and check its plan line. **It must be the identical
   `1..N`.** Test numbers shift whenever tests are added or removed, so a
   different `N` means the numbering has moved — discard that run and try
   another.
3. Double-check by comparing the last few test names *before* the stall in both
   logs. If they don't match, the numbering moved anyway; find another run.
4. Look up the next number after the last one that finished. That is the test
   that never reported.

This works only because bats prints results in numerical order here, so "last
was `ok N`" really does mean `N+1` is where it stopped. Confirm that before
relying on it: the first dozen numbers in the reference log should climb
strictly 1, 2, 3, …. If they jump around, the suite ran its tests in parallel,
and the last number printed tells you nothing about where it stopped.

The reference log also gives the test's **healthy duration**, which is worth
reporting and is often decisive: a test that normally takes 100 ms is a very
unlikely candidate to have hung for half an hour, and that mismatch is a signal
the job belongs in 3b rather than 4a.
