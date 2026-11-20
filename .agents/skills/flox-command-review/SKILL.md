---
name: flox-command-review
description: Review new or changed commands in flox/flox against the repository's command conventions and produce an evidence-based checklist. Use for "review this Flox command" or "check command conventions". Currently checks command-group help fallback only; not a general CLI standards audit or a command implementation guide.
allowed-tools: Read, Grep, Glob, Bash
---

# Flox command review

Review a command change and report a checklist with evidence and appropriate fixes.

Invoke as `/flox-command-review` in Claude Code or `$flox-command-review` in
Codex, with the command or change to review in the request or conversation.
This is an invoked review, not a Git hook or a deterministic linter.

## Workflow

1. **Establish the review target.** Use the requested commands, files, or diff.
   Otherwise inspect the current branch's changes against its base, including
   staged and unstaged changes and relevant untracked files. Identify the
   actual user-facing commands affected, including parent groups when a child
   changes. State the target and revision or working-tree state reviewed.
   If no command change is present, report that instead of auditing unrelated
   commands. Ask only if the target cannot be resolved from the request and
   repository state.
2. **Read applicable repository instructions.** Follow AGENTS.md and
   CONTRIBUTING.md in the reviewed checkout. Compare with maintained commands
   there; distinguish an explicit requirement from an established convention.
   A precedent is evidence, not permission to copy deprecated code.
3. **Select checks using the table below.** Inspect the target's parser and
   handler enough to decide applicability. Load a detailed check only when its
   condition holds or applicability is uncertain. Do not recursively read or
   search this skill's directories: that would preload conditional guidance.
4. **Run each applicable check.** Establish a failure from source or runtime
   evidence before opening its repair reference. Load repair guidance only
   for a failed check; do not load it for passing or inapplicable checks.
   Keep this review read-only unless the user separately requested fixes.
5. **Report the checklist.** Include one row per reviewed command and check,
   using the format below. State limitations and the next corrective action.

## Check index

| Check | Load condition | Detailed check |
|---|---|---|
| `command-group-help` | A command selects child subcommands without an intentional default action, or its bare-invocation behavior is uncertain. Exclude leaf commands and confirmed useful defaults. | [Command-group help](checks/command-group-help.md) |

Only this check is implemented. Do not imply that naming, module layout,
telemetry, or other command standards have passed. Add future checks as
separate conditional entries with their own details and failure-only repairs.

## Checklist output

| Command | Check | Status | Evidence and next action |
|---|---|---|---|

Use `PASS`, `FAIL`, `N/A`, or `NEEDS JUDGMENT`. Give each `N/A` a reason.
For failures, identify the file and line or command output, explain the
behavioral consequence, and suggest the smallest correction from the repair
reference. For uncertainty, state the missing evidence and how to obtain it.
Distinguish source review from executed validation: list commands actually
run, exit statuses, and relevant output. Never report an unrun runtime check
as passed. A source-only pass must explicitly say runtime is unverified.

## Resolving references

Skill links resolve relative to the file containing them. Repository source
links in the detailed check and repair reference resolve to this checkout.
For lexical path calculations, resolve the real path of SKILL.md first so
the shared Claude Code symlink still reaches the same source. When reviewing
another checkout or a supplied snapshot, inspect that target's corresponding
files rather than treating this skill's checkout as the reviewed source.
If copied without repository sources, locate the target checkout from the
request; missing source is an evidence gap, not a passing result.
