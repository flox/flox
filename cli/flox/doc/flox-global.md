---
title: FLOX-GLOBAL
section: 1
header: "Flox User Manuals"
...

# NAME

flox-global - manage commands available without activating an environment

# SYNOPSIS

```text
flox global install [--id <id>] <package>... [--dry-run] [--json]
flox global list [--commands] [--json]
flox global upgrade [<package-or-group>...] [--all] [--dry-run] [--json]
flox global uninstall <package>... [--dry-run] [--json]
flox global retry-publication [--dry-run] [--json]
flox global dir [--bin | --sbin] [--json]
flox global which <command> [--json]
flox global update-shell [--shell <bash|zsh|fish|tcsh>] [--print-script] [--json]
```

# DESCRIPTION

Global packages live in one ordinary environment at
`<Flox data directory>/environments/global`. Package requests, version
requirements, groups, resolution, builds and retention use ordinary Flox
environment semantics. Reinstalling an existing request does not upgrade it.
Use `global dir` to discover this environment independently of the current
working directory.

Flox publishes executable entries from the environment's public `bin` and
`sbin` trees to the corresponding directories under the Flox data directory.
These links point through the ordinary environment's run link, preserving
package runtime wrappers. Global invocation does not activate hooks, variables,
profiles or services. A package without public CLI commands is reported as an
error; the result separately reports any ordinary installation that occurred.

`list` shows installed requests; `list --commands` shows commands, observed
providers and publication status. Requester attribution can be unresolved;
it does not confer exclusive ownership of a shared supplier. `which` also
reports the executable selected by the calling process's inherited PATH.
Parent-shell aliases, functions and command caches are outside that observation.
Package IDs take precedence over command-name targets; an unambiguous observed
provider may also identify a package for upgrade or uninstall.

## Shell setup

Installation prints setup instructions. Run `global update-shell` explicitly
to add an idempotent marked PATH block to startup files, then open a new shell.
Use `--shell` to choose a shell or `--print-script` to inspect and apply its
script yourself. With `--json`, the script is a field in the result object.

The block appends global `bin` followed by `sbin`, removing duplicate occurrences
of those two directories. Existing PATH entries retain precedence, including an
active project environment or an external command of the same name. Setup is
PATH-only. Startup-file symlinks and ambiguous marked blocks are preserved and
reported for explicit editing. A failure can leave earlier files updated;
the result lists the completed files.

## Publication and recovery

Flox records created command links in
`<Flox data directory>/global-publication/state.toml`. This generated versioned
file is an ownership receipt, not package authority. A missing file grants no
ownership; invalid, unreadable or unsupported state stops publication mutation.
Do not delete it to repair publication.

Existing foreign files, links and ambiguous replacements are preserved. Inspect
the reported destination and observed target, move a conflicting entry aside
only if appropriate, and run `global retry-publication`. Retry reconciles the
**current** ordinary output without reinstalling, upgrading, fetching or building.
If the ordinary output or metadata is unavailable or inconsistent, recover the
ordinary environment first. Retry does not infer removal from missing output.

Each created or removed link is followed by a receipt checkpoint. Interruption
after creation but before checkpointing leaves an unowned link that retry
preserves and reports. Interruption after removal permits retry to clear an
obsolete receipt or recreate a currently desired missing link.

The publication lock serializes cooperating publishers only, and is never held
through an ordinary package build. Existing names follow ordinary run-link
updates immediately, including before publication completes. Publication can be
partial, and detected output movement requires retry. Inspection followed by
removal is not an atomic identity-conditional operation against unrelated
writers; inode reuse and unobserved change-and-revert races are outside this
bounded mechanism. There is no batch atomicity, saved-output rollback,
power-loss durability or extra retention promise for running programs.

## Dry run and JSON

Package `--dry-run` uses isolated manifest/lock copies and scratch build links.
It can resolve, fetch and build into caches, but preserves live package files,
publication links/receipts and shell files. Retry dry-run only observes the
current output and planned publication changes.

`--json` writes one result object to stdout, including runtime and usage errors.
Version 1 includes `schema_version`, `operation`, `dry_run`,
`environment_directory`, `environment_outcome`, `before`, `after`,
`package_state_before`, `package_state_after`, `publication`, `path`,
`directory`, `selected_by_path`, `shell` and `errors`.
Snapshots separate ordinary packages, actual output commands and recorded links.
Publication reports planned actions, completed paths, observed/current output
and errors with stable categories and affected paths.

Exit status is zero only when the requested ordinary operation succeeds or is
unchanged and required publication completes. Ordinary success with incomplete
publication returns nonzero. Missing PATH entries and external shadowing do
not make installation fail. A killed process cannot emit a final result.

Global flag routing (`-g`), command selection/rename/overwrite/doctor, imports,
remote association/history and global activation are not part of this namespace.

# SEE ALSO

[`flox-install(1)`](./flox-install.md),
[`flox-upgrade(1)`](./flox-upgrade.md),
[`flox-uninstall(1)`](./flox-uninstall.md),
[`manifest.toml(5)`](./manifest.toml.md)
