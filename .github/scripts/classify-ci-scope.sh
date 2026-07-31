#!/usr/bin/env bash
# ============================================================================ #
#
# Classify the change under test as "heavy" or not.
#
# A heavy change needs the full build and test fleet — including the remote
# Darwin builders, which are a scarce shared resource. A change that only
# touches documentation or repository chrome does not.
#
# The classification is default-deny: `heavy=true` unless *every* changed file
# matches the inert allowlist below. Every error path also lands on
# `heavy=true`. An over-run costs CI minutes; an under-run costs correctness,
# so the two are not symmetric and this script always errs towards running.
#
# Consumed by the `scope` job in .github/workflows/ci.yml, which exposes the
# result as `needs.scope.outputs.heavy`.
#
# Run `./.github/scripts/classify-ci-scope.sh --classify-only` and feed it
# newline-separated paths on stdin to exercise the allowlist without git.
#
# ---------------------------------------------------------------------------- #
#
# Why this is not `dorny/paths-filter`.
#
# That action answers "did *any* changed file match this pattern?". This gate
# needs the opposite quantifier — "did *every* changed file match the inert
# allowlist?" — and the two are not interchangeable, because getting the
# polarity backwards fails towards skipping CI rather than towards running it.
#
# The action can be coerced into the universal form with
# `predicate-quantifier: every` over a list of negated globs, but the two rules
# that carry the actual risk do not fit in a filter block at all: the
# `cli/flox/doc/*` carve-out below is a disjunction, and "an empty diff is
# heavy" is not a pattern. Both would end up in a hand-written boolean over
# three filter outputs in the job's `outputs:`, which is exactly the logic that
# most needs a test and is the one place no test can reach. The
# never-exit-non-zero posture would still have to be added on top.
#
# So the safety-relevant surface would not shrink, it would move somewhere less
# legible and stop being testable. `--classify-only` above is what buys the
# allowlist a test suite that runs without CI.
#
# ---------------------------------------------------------------------------- #

set -uo pipefail

# ---------------------------------------------------------------------------- #

# Report the verdict and stop. Always exits 0: a non-zero exit would fail the
# `scope` job, which would in turn skip every job that depends on it — and a
# required check that never posts a status leaves the merge queue waiting
# forever.
emit() {
  local heavy="$1" reason="$2"

  echo "heavy=$heavy (reason: $reason)" >&2
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    echo "heavy=$heavy" >> "$GITHUB_OUTPUT"
  fi
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    echo "\`heavy=$heavy\` — $reason" >> "$GITHUB_STEP_SUMMARY"
  fi
  echo "heavy=$heavy"
  exit 0
}

# ---------------------------------------------------------------------------- #

# Does this path, on its own, leave the heavy jobs with nothing to do?
#
# Anything not listed here is heavy, so a newly added workflow, action, or
# source directory fails safe rather than silently opting out of CI.
is_inert() {
  case "$1" in
    # Not inert despite being markdown: pkgs/flox-manpages compiles every
    # `*.md` under this directory into a manpage that is bundled into the
    # `flox` package, so these are build inputs.
    cli/flox/doc/*) return 1 ;;

    *.md) return 0 ;;
    docs/*) return 0 ;;

    .github/ISSUE_TEMPLATE/*) return 0 ;;
    PULL_REQUEST_TEMPLATE*) return 0 ;;
    .github/PULL_REQUEST_TEMPLATE*) return 0 ;;

    # Workflows that only govern bots and labelling. Deliberately *not*
    # ci.yml, nix-managed-lints.yml or build-examples-tests.yml, which decide
    # what CI itself does.
    .github/workflows/claude-review.yml) return 0 ;;
    .github/workflows/claude-mention.yml) return 0 ;;
    .github/workflows/auto-label.yml) return 0 ;;

    CODEOWNERS | .github/CODEOWNERS) return 0 ;;
    LICENSE | LICENSE.*) return 0 ;;

    *) return 1 ;;
  esac
}

# Read newline-separated paths on stdin and emit the verdict.
classify() {
  local path
  local -i seen=0

  while IFS= read -r path; do
    [ -n "$path" ] || continue
    seen+=1
    if ! is_inert "$path"; then
      emit true "'$path' is not on the inert allowlist"
    fi
  done

  # An empty diff is not a licence to skip: it more likely means the range was
  # computed wrongly than that a pull request changed nothing.
  if [ "$seen" -eq 0 ]; then
    emit true "the computed diff is empty"
  fi

  emit false "all $seen changed files are documentation or repository chrome"
}

# ---------------------------------------------------------------------------- #

# Print the changed paths for this event, or return non-zero if the range
# cannot be established.
#
# `--no-renames` lists both the old and the new path of a rename, so a file
# moved out of the allowlist is still seen.
changed_files() {
  local base head

  case "${GITHUB_EVENT_NAME:-}" in
    merge_group)
      base="${MERGE_GROUP_BASE_SHA:-}"
      [ -n "$base" ] || return 1
      git cat-file -e "${base}^{commit}" 2> /dev/null ||
        git fetch --no-tags origin "$base" || return 1
      git diff --no-renames --name-only "${base}...HEAD" || return 1
      ;;

    pull_request | pull_request_target)
      # `github.event.pull_request.base.sha` is the base branch tip as of when
      # the pull request was last synchronised, so it drifts as the base branch
      # moves and would report unrelated files. Use the merge base instead.
      #
      # The head branch may live in a fork, so it is not guaranteed to exist on
      # `origin`. Fetch it through `refs/pull/<number>/head`, which GitHub
      # maintains on the base repository for every pull request.
      [ -n "${PR_BASE_REF:-}" ] && [ -n "${PR_NUMBER:-}" ] || return 1
      git fetch --atomic --no-tags --force origin \
        "refs/heads/${PR_BASE_REF}:refs/ci-scope/base" \
        "refs/pull/${PR_NUMBER}/head:refs/ci-scope/head" || return 1
      base="$(git merge-base refs/ci-scope/base refs/ci-scope/head)" || return 1
      head="$(git rev-parse refs/ci-scope/head)" || return 1
      git diff --no-renames --name-only "$base" "$head" || return 1
      ;;

    *) return 1 ;;
  esac
}

# ---------------------------------------------------------------------------- #

main() {
  local files

  if [ "${1:-}" = "--classify-only" ]; then
    classify
    return
  fi

  # `push`, `schedule` and `workflow_dispatch` runs are the record of what main
  # actually builds, and a manual dispatch is by definition someone asking for
  # the full suite. They are never scoped down.
  case "${GITHUB_EVENT_NAME:-}" in
    push | schedule | workflow_dispatch)
      emit true "the '${GITHUB_EVENT_NAME}' event always runs the full suite"
      ;;
  esac

  if ! files="$(changed_files)"; then
    emit true "could not compute the diff range for the '${GITHUB_EVENT_NAME:-unknown}' event"
  fi

  classify <<< "$files"
}

main "$@"

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
