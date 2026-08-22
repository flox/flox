---
name: investigating-merge-queue-failures
description: Use when investigating flaky CI in flox/flox — triaging failures in the GitHub merge queue, working out whether a CI failure is a flaky test, slow infrastructure, or a real break. Accepts an optional lookback window (defaults to 30 days). Examples - "/investigating-merge-queue-failures", "/investigating-merge-queue-failures 14d", "why is the merge queue red so often?"
argument-hint: "[lookback window, e.g. 30d (default) or 7d]"
---

# Investigating merge queue failures

A merge queue run tests the PR *as it would land*, on top of everything ahead of
it. The PR's own CI already passed before it was queued, so a merge-queue
failure is usually — but not always — a flake.

The goal of this skill is to sort each failure into a category and, where the
category is new, add it to `references/failure-categories.md`. **That reference
is the durable output.** Do not write occurrence logs, per-run tallies, or dated
findings to disk; those go in the chat reply and are thrown away.

## Do not assume every merge-queue failure is flaky

Two non-flaky things also fail here, and mislabelling them wastes real
debugging time. Both exist because the queue tests something the PR's own CI
never did:

- **A genuine semantic conflict between PRs.** The queue builds the PR on top
  of everything ahead of it in the group, so two PRs that each pass alone can
  break when combined. This is the queue working as designed.
- **A check that is deliberately stricter in the queue.** The commitizen hook
  narrows its allowed commit prefixes when `IS_MERGE_QUEUE=1`, so a branch
  carrying a `fixup!` commit passes PR CI and fails the queue every time. Any
  check that reads `IS_MERGE_QUEUE` can do this.

`references/failure-categories.md` has the signatures that separate these from
flakes. Check it before concluding anything is flaky — and note that neither
is identified by failing more than once. See the note on repeats in step 1.

## Setup

The skill ships a Flox environment providing `gh`, `jq`, and GNU `coreutils`.
Use it — the timestamp arithmetic in step 4 needs GNU `date -d`, and `jq`'s
`--arg` is not available through `gh api --jq`. Nothing here should assume the
host has `jq`, `sed`, `awk`, `python3`, or `perl`.

Steps 1–3 are mechanical, so they are scripted:

```bash
flox activate -d <skill dir> -- <skill dir>/scripts/collect.sh [since] [outdir]
```

`since` defaults to 30 days ago, `outdir` to `./merge-queue-triage`. It writes
`jobs.tsv` — one row per failed job, giving the bats plan line (`1..N`, the
total number of tests), the number of the last test that finished, the timeout
gap in seconds, and whether a `124` appears anywhere — and leaves the raw logs
in `logs/` for the judgement calls it deliberately does not make.

**The script collects; it does not classify.** Read `jobs.tsv`, then work
through step 4 with `references/failure-categories.md` in hand. The rest of
this document explains what the script does and how to go further by hand when
a job does not fit the mould.

## Step 1: Collect failed runs

```bash
gh run list --repo flox/flox --event merge_group --status failure \
  --limit 100 \
  --json databaseId,workflowName,headBranch,createdAt \
  --jq '.[] | select(.createdAt > "<WINDOW_START>") | "\(.databaseId)\t\(.createdAt)\t\(.workflowName)\t\(.headBranch)"'
```

`--event merge_group` is what restricts this to the queue; those runs have a
`gh-readonly-queue/main/pr-<N>-<sha>` head branch. Substitute an ISO date for
`<WINDOW_START>`.

Two things to note about the run list:

- **Ignore `cancelled`.** When one run in a merge group fails, GitHub dequeues
  the group and cancels its siblings. Those cancellations are bookkeeping, not
  failures.
- **Repeat failures on the same `pr-<N>-<sha>` prove nothing on their own.**
  It is tempting to read "failed twice, therefore reproduces, therefore real",
  but bogged-down infrastructure fails the same way on consecutive attempts,
  and an established flaky test reproduces perfectly well. Group the repeats by
  *signature* and classify that; the repetition itself is not evidence.

For a denominator, count all merge-queue runs in the window (drop
`--status failure` and group by conclusion). A category is only worth chasing
relative to how often the queue runs at all.

## Step 2: Collect failed jobs and their logs

A run has many jobs; only some failed.

```bash
gh api "repos/flox/flox/actions/runs/<RUN_ID>/jobs?per_page=100" \
  --jq '.jobs[] | select(.conclusion=="failure" or .conclusion=="timed_out") | "\(.id)\t\(.name)\t\(.conclusion)"'
```

Then fetch each job's full log:

```bash
gh api "repos/flox/flox/actions/jobs/<JOB_ID>/logs" > <JOB_ID>.txt
```

Practical notes:

- Write logs to the scratchpad directory, not the worktree. A month of
  failures is a few MB.
- Logs are retained ~90 days, so a 30-day window is always available.
- Every line is prefixed with a 29-character ISO timestamp. Strip it with
  `cut -c30-`.
- `gh api --jq` does **not** accept `--arg`. Pipe to the environment's `jq`
  instead of working around it.
- Anchor any `1..N` search on the timestamp prefix
  (`^[0-9T:.Z-]+ 1\.\.[0-9]+`). Unanchored, it matches unrelated ranges
  elsewhere in the log and reports a plan the suite never had.

## Step 3: Extract a signature

**The `##[error]` line is almost never the signature.** The large majority of
failed jobs report only `Process completed with exit code 1`. Get it for the
exit code, then look deeper:

```bash
grep -a "##\[error\]" <JOB_ID>.txt | cut -c30- | sort -u
```

| Failure kind | What to grep for |
|---|---|
| bats test | `not ok <N> <name>`, then `grep -a -A 20` that line for the assertion |
| bats suite shape | the plan line `1..<N>`, and the last `ok`/`not ok` line |
| a timeout | the timestamp gap between the last log line and the `##[error]` |
| cargo/nextest | `panicked at <file>:<line>`, `test result: FAILED` |
| nix build | `error: builder for '/nix/store/...' failed`, and the `>`-prefixed build output |
| network | `Could not resolve hostname (6)`, `Timeout was reached (28)` — curl exit codes |
| — | check whether the line says `warning:` or `error:` before calling it the cause |
| timeout inside a test | `failed with status 124`, `status : 124` |

## Step 4: Classify — in this order

Order matters, because the loudest symptom is usually downstream of the real
cause. Work through `references/failure-categories.md`, but always:

1. **For a timed-out job, measure the gap before calling it a hang.** Compare
   the timestamp of the last log line to the `##[error]` line. Tens of minutes
   means something genuinely hung. A few seconds means the suite was still
   running when the 30-minute budget expired — that is slow infrastructure,
   and there is no hung test to look for. Skipping this check invents hangs
   that never happened.
2. **Look for `124`, even in a job that timed out.** A `timeout` expiry inside
   a test is the root cause; a job timeout that follows it is a symptom.
3. **Check the plan line against the last test.** If a job hung and its plan is
   `1..N` with last line `ok N`, every test ran and the *suite finished* — the
   job hung afterwards. That is the FD 3 hang, not a hung test.
4. **Check whether the same test failed on every platform in the run.** That is
   the semantic-conflict signature, not a flake.
5. **For infrastructure, identify the host before choosing a category.**
   GitHub and `releases.nixos.org` are not ours — retry and exclude them
   (section 2). Our builders, our cache, and the tailnet that reaches them are
   (section 3). The response differs, so the report has to.
6. Only then classify by the individual test signature (section 4).

A job can belong to more than one category — a slow builder causing a 124,
causing a failed test, causing an FD 3 hang, is one job with three symptoms.
Tag per job and say so; don't force one label per run.

If a test hung or was cut off mid-flight, its name was never printed. See
"Naming a test that never printed" in the reference for how to recover it from
a healthy run.

## Step 5: Report and update the catalog

Report in chat, and lead with the split that says what to prioritise:

- **section 3 (builder instability)** vs **section 4 (flaky tests)** — this is
  the whole point of the report. It answers whether the next block of effort
  belongs in the fleet or in the test suite, which a single combined "flaky"
  number cannot;
- **section 1** separately, since it is not flakiness and needs a code fix;
- **section 2** as a footnote, with a note that it is excluded from the
  totals — counting outages we cannot fix only inflates the number.

Then give the platforms each skews toward and which categories share a root
cause. Name the specific tests inside a flaky cluster so they can be fixed, but
keep run IDs and dates out of the reference file.

If a signature matched nothing in the catalog, propose a new category —
signature, what it means, and how to tell it apart from its neighbours — and
add it to `references/failure-categories.md`. That is the only file in the
repository this skill writes.

**Only add a category the evidence supports.** A category earns its place by
having failed something. Before adding one, confirm the signature appears as an
`error:` and not merely a `warning:`, and that it was the reason a job failed
rather than noise in a step that passed. A category invented from warnings
sends every future reader hunting a problem that does not exist.

## Step 6: Write an HTML report

After classifying, write a standalone HTML file to `<outdir>/report.html` —
self-contained, with the CSS inlined and no external requests, so it can be
opened straight from disk or attached to an issue. The report is a snapshot,
not state: it is regenerated per run, lives with the collected logs rather than
in the repository, and nothing in it feeds back into the catalog.

It must contain:

- **The denominator, up front.** Failed runs over total merge-queue runs in the
  window. "27 failures" means nothing without "out of 186 runs".
- **A pie chart of the four categories**, with each slice's percentage and job
  count. Four slices of a genuine part-to-whole is what a pie is for; keep it to
  the four categories and do not split it further. Give each slice the same
  colour that category's section uses, so identity holds down the page, and put
  the names in a legend — never colour alone.
- **The architecture split within categories 3 and 4**, as a percentage for
  each, since where the failures land is most of the diagnosis. Use steps of the
  owning category's colour rather than new hues, so the split reads as a
  subdivision. Note that `macos-14-xlarge` runners are `macos-14-arm64`, so
  those jobs are aarch64-darwin — reading the label as x86 miscounts the split.
- **Sections 3 and 4 as the two headline numbers**, side by side and clearly
  distinguished — builder instability versus flaky tests. This is the report's
  entire reason for existing: it says whether the next block of effort should go
  into the fleet or into the test suite. Do not merge them into one "flaky"
  figure, which cannot answer that.
- **Section 1 separately**, marked as needing a code fix rather than CI work.
- **Section 2 as a footnote**, with its count stated and explicitly excluded
  from the totals, so the exclusion is visible rather than silent.
- **Named tests** inside each category-4 cluster, and named hosts or steps
  inside each category-3 one. A category with no specifics in it cannot be
  acted on.
- **Root causes that span categories**, called out — most importantly that a
  `124` in a category-4 job usually means the real fix is in category 3.
- **Anything uncategorisable**, counted honestly rather than forced into the
  nearest bucket, with the reason it could not be classified.

**Never print an aggregate next to the breakdown that supersedes it.** "76% of
failed jobs were darwin" is worth nothing once the per-category split is on the
page, and repeating it as a headline figure buys prominence with no
information. Where both would fit, keep the breakdown and delete the aggregate.

Keep dates and run IDs to this report; they must not reach the catalog. Link
each cluster to a failing job so a reader can check the evidence, and say
plainly which findings rest on a single occurrence.

The palette is chosen, not incidental: the category colours pass a
colourblindness and contrast check in both light and dark, and the dark steps
are separate values rather than a flip of the light ones. If you restyle the
report, re-validate rather than picking new colours by eye — a teal that looks
fine can sit under the chroma floor and read as grey, and dark steps drift
above the dark lightness band easily.
