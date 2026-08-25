#!/usr/bin/env bash
# Collect failed merge-queue jobs and summarise each one.
#
# Run inside the skill's flox environment, which provides gh, jq and GNU date:
#   flox activate -d <skill dir> -- ./scripts/collect.sh [since] [outdir]
#
# Emits <outdir>/jobs.tsv, one row per failed job, and <outdir>/conclusions.tsv,
# the run counts by conclusion that the report's denominator comes from. Leaves
# the raw logs in <outdir>/logs/ for the judgement calls it does not make.
set -uo pipefail

REPO="${REPO:-flox/flox}"
SINCE="${1:-$(date -d '30 days ago' +%Y-%m-%d)}"
OUT="${2:-./merge-queue-triage}"
# Only a bound on the reply, not the window: `--created` does the filtering
# server-side. Raise it if the truncation warning below ever fires.
LIMIT="${LIMIT:-1000}"

# GNU `date -d` is what computes the default. Without it the substitution above
# yields an empty string, which `--created '>='` accepts and answers with zero
# runs — a wrong window that reads exactly like a quiet month.
: "${SINCE:?empty window — run inside the flox environment this skill ships, which provides GNU date}"

mkdir -p "$OUT/logs"

# The GitHub API fails transiently often enough to matter here. Retry, and if
# it still fails, say so loudly — a silently dropped run or log understates a
# category, which is worse than no answer at all.
try() { # try <destination> <command>...
  local dest="$1"; shift
  for attempt in 1 2 3; do
    if "$@" > "$dest" 2>/dev/null && [ -s "$dest" ]; then return 0; fi
    [ "$attempt" -lt 3 ] && sleep $(( attempt * 3 ))
  done
  return 1
}

try_api() { try "$2" gh api "$1"; }

run_list() { # run_list <destination> <extra gh run list args>...
  local dest="$1"; shift
  try "$dest" gh run list --repo "$REPO" --event merge_group \
    --created ">=$SINCE" --limit "$LIMIT" "$@" || return 1
  # `gh` returns exactly `--limit` rows when it truncates and says nothing
  # about it, so a capped query reads as a quiet month rather than an error.
  if [ "$(jq 'length' < "$dest")" -ge "$LIMIT" ]; then
    printf 'WARNING: hit --limit %s; older runs in the window are MISSING. Re-run with LIMIT=%s\n' \
      "$LIMIT" "$(( LIMIT * 4 ))" >&2
  fi
}

# ---------------------------------------------------------------------------
# The denominator, collected first because the rest is meaningless without it:
# "27 failures" says nothing until it is "27 out of 168 runs".
# ---------------------------------------------------------------------------
if run_list "$OUT/.all.json" --json conclusion; then
  jq -r 'group_by(.conclusion)[] | [ (.[0].conclusion // "pending"), length ] | @tsv' \
    < "$OUT/.all.json" > "$OUT/conclusions.tsv"
  printf 'merge-queue runs since %s: %s\n' "$SINCE" "$(jq 'length' < "$OUT/.all.json")" >&2
  rm -f "$OUT/.all.json"
else
  : > "$OUT/conclusions.tsv"
  printf 'WARNING: could not count all merge-queue runs — the report has NO denominator\n' >&2
fi

# ---------------------------------------------------------------------------
# Failed runs in the window. Only `failure` and `timed_out` matter: when one
# run in a merge group fails, GitHub cancels its siblings, and those
# cancellations are bookkeeping rather than failures.
# ---------------------------------------------------------------------------
if ! run_list "$OUT/.runs.json" --status failure \
    --json databaseId,workflowName,headBranch,createdAt
then
  printf 'FATAL: could not list failed merge-queue runs after retries — nothing collected\n' >&2
  exit 1
fi

jq -r '
    .[]
    | [ .databaseId,
        .createdAt,
        .workflowName,
        ( .headBranch | capture("pr-(?<n>[0-9]+)") // {n:"?"} | .n )
      ] | @tsv' < "$OUT/.runs.json" > "$OUT/runs.tsv"
rm -f "$OUT/.runs.json"

printf 'failed runs: %s\n' "$(wc -l < "$OUT/runs.tsv")" >&2

# ---------------------------------------------------------------------------
# Failed jobs within those runs, plus each job's full log.
# ---------------------------------------------------------------------------
: > "$OUT/jobs.raw"
failures=0
while IFS=$'\t' read -r rid created wf pr; do
  if try_api "repos/$REPO/actions/runs/$rid/jobs?per_page=100" "$OUT/.jobs.json"; then
    jq -r --arg rid "$rid" --arg created "$created" --arg wf "$wf" --arg pr "$pr" '
        .jobs[]
        | select(.conclusion=="failure" or .conclusion=="timed_out")
        | [$rid, $created, $wf, $pr, (.id|tostring), .name, .conclusion] | @tsv' \
      < "$OUT/.jobs.json" >> "$OUT/jobs.raw"
  else
    printf 'WARNING: could not list jobs for run %s — it is MISSING from the output\n' "$rid" >&2
    failures=$((failures + 1))
  fi
done < "$OUT/runs.tsv"
rm -f "$OUT/.jobs.json"

while IFS=$'\t' read -r rid created wf pr jid jname concl; do
  [ -s "$OUT/logs/$jid.txt" ] && continue
  if ! try_api "repos/$REPO/actions/jobs/$jid/logs" "$OUT/logs/$jid.txt"; then
    rm -f "$OUT/logs/$jid.txt"
    printf 'WARNING: could not fetch log for job %s (%s) — its row will be blank\n' "$jid" "$jname" >&2
    failures=$((failures + 1))
  fi
done < "$OUT/jobs.raw"

printf 'failed jobs: %s\n' "$(wc -l < "$OUT/jobs.raw")" >&2
if [ "$failures" -gt 0 ]; then
  printf 'INCOMPLETE: %s fetch(es) failed after retries; re-run before trusting counts\n' \
    "$failures" >&2
fi

# ---------------------------------------------------------------------------
# Summarise. Every field here is mechanical; classification is not, and is
# left to the reader with references/failure-categories.md in hand.
# ---------------------------------------------------------------------------
epoch() { date -d "$1" +%s 2>/dev/null || echo 0; }

{
printf 'date\tpr\tjob_name\tplan\tlast_test\tgap_s\thas_124\tsignature\n'
while IFS=$'\t' read -r rid created wf pr jid jname concl; do
  log="$OUT/logs/$jid.txt"

  # Anchor on the timestamp prefix: an unanchored "1..N" matches unrelated
  # ranges elsewhere in the log.
  plan=$(grep -a -m1 -E '^[0-9T:.Z-]+ 1\.\.[0-9]+' "$log" 2>/dev/null \
         | cut -c30- | grep -oE '^1\.\.[0-9]+')
  last_line=$(grep -a -E '^[0-9T:.Z-]+ (ok|not ok) [0-9]+ ' "$log" 2>/dev/null | tail -1)
  last_test=$(printf '%s' "$last_line" | cut -c30- | grep -oE '^(ok|not ok) [0-9]+' | grep -oE '[0-9]+$')

  # Gap between the last line of real output and the timeout error. Tens of
  # minutes means something hung; seconds means the suite simply ran out of
  # the 30-minute budget. Empty when the job did not time out.
  gap=""
  err_ts=$(grep -a -m1 'has timed out after' "$log" 2>/dev/null | cut -c1-28)
  if [ -n "$err_ts" ]; then
    prev_ts=$(grep -a -B 1 'has timed out after' "$log" 2>/dev/null | head -1 | cut -c1-28)
    gap=$(( $(epoch "$err_ts") - $(epoch "$prev_ts") ))
  fi

  # A timeout(1) expiry often explains both a failed test and a hang in the
  # same job, so surface it even when a louder symptom is present.
  has_124=no
  grep -a -qE 'status ?:? 124|exit code 124' "$log" 2>/dev/null && has_124=YES

  sig=$(grep -a -m1 -E '^[0-9T:.Z-]+ not ok [0-9]+ ' "$log" 2>/dev/null | cut -c30-110)
  [ -z "$sig" ] && sig=$(grep -a -m1 '##\[error\]' "$log" 2>/dev/null | cut -c30-110)

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "${created%T*}" "$pr" "$jname" "${plan:--}" "${last_test:--}" "${gap:--}" "$has_124" "${sig:--}"
done < "$OUT/jobs.raw"
} > "$OUT/jobs.tsv"

printf 'wrote %s\n' "$OUT/jobs.tsv" >&2
