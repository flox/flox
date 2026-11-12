## Makefile conventions (`package-builder/flox-build.mk`)

GNU make's dependency semantics express most lifecycle logic on their
own. If a recipe contains shell logic deciding whether its own work
should happen, the rule is probably modeling the wrong thing — reach
for a make feature first:

- **Create-if-missing artifact:** a file target with no prerequisites.
  The recipe runs only when the file is absent; an existing file is
  authoritative. No `[ -f ... ]` recipe guards.
- **Temp file the build may create for itself:** mark it
  `.INTERMEDIATE`, unconditionally. make deletes an intermediate it
  created at the end of the run (and on fatal signals) and leaves a
  pre-existing file untouched — that distinction needs no flag
  variables and no `rm` bookkeeping.
- **Steps that must run on every invocation** are `.PHONY` themselves
  (see the next bullet) — phony-ness does not propagate, so transitive
  reach through the phony `check-build-prerequisites` root is not
  enough. The root's job is validation and ordering; do not add it as a
  direct prerequisite of targets that already reach it — redundant
  edges obscure the graph.
- **A real file that must be regenerated unconditionally** (the
  per-invocation build chain: source lists/tarballs, build caches,
  rendered build scripts, evals, build outputs, result files): mark the
  target `.PHONY`. make then triggers the recipe irrespective of the
  file's presence or timestamp, so the rule can never tie with
  prerequisites written in the same second. Phony-ness does not
  propagate — a phony prerequisite forces only its direct dependent,
  whose own output is again an ordinary file — so mark every link of a
  must-regenerate chain, not just its head. Prefer this over a
  `FORCE`-style empty prerequisite, which an out-of-band file named
  `FORCE` would defeat. Cross-invocation caching is nix's job, not
  make's.
- **Returning data to the CLI:** fold it into `build-meta.json`
  (`jq --slurpfile`), never hand back a path for the caller to read
  and clean up. Each subsystem cleans up the files it creates.
- `mkVarname`'s define body carries leading whitespace: `$(strip ...)`
  the result, or launder it through an assignment, before interpolating
  it into a variable name.
