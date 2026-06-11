# Goal command

Execute the architecture analysis plan defined in `GOAL.md` at the repository
root.

Arguments: $ARGUMENTS

## How to interpret arguments

- No arguments or `all`: run every workstream that does not yet have an
  output file in `docs/architecture-analysis/`, in the dependency order
  given by the "Sequencing" section of GOAL.md, then write `REPORT.md`.
- A single workstream letter (`A`, `B`, `C`, `D`, `E`, `F`): run only that
  workstream. Warn if its dependencies (per the Sequencing section) have no
  output file yet, and proceed using whatever evidence is gatherable.
- `report`: write or rewrite `REPORT.md` from the workstream outputs that
  exist. If outputs are missing, list them and ask whether to proceed with
  partial coverage.
- `status`: do not run anything; report which outputs exist, when they were
  last touched (git log), and which workstreams remain.

## Execution rules

1. Read `GOAL.md` first and follow its method, output spec, and ground rules
   for the selected workstream exactly. GOAL.md is the source of truth; this
   command file only handles invocation.
2. **Analysis only.** Write only inside `docs/architecture-analysis/`. Never
   modify production code, Cargo.toml files, CI, or CODEOWNERS — proposals
   for those belong inside the output documents.
3. Ground every claim in evidence: file paths (with line numbers where
   useful) or reproducible commands. End every output with a "How to
   reproduce" section.
4. Where GOAL.md says input is needed (e.g. floxhub/floxdash capabilities in
   Workstream B) and none was provided, use the documented default
   assumptions and record them in the output's "Assumptions" section.
5. Use subagents for broad evidence-gathering (per-command metrics,
   dependency tracing) and run independent workstreams in parallel where the
   sequencing allows.
6. `REPORT.md` must follow the "Final Output — REPORT.md" spec in GOAL.md
   exactly: executive summary, current-vs-target ASCII diagrams, one section
   per output with Context / What the data says / Impact / Example /
   Diagram, decision list, phased backlog, and assumptions.
7. After completing the requested scope, summarize what was produced, where,
   and what remains, then stop. Do not begin implementing any of the
   recommendations.
