---
title: FLOX-SANDBOX
section: 1
header: "Flox User Manuals"
...

# NAME

flox-sandbox - review and manage sandbox grants for an `ask`-mode activation

# SYNOPSIS

```text
flox [<general-options>] sandbox
     [-d=<path> | -r=<owner/name>]

flox [<general-options>] sandbox list
     [-d=<path> | -r=<owner/name>]

flox [<general-options>] sandbox allow
     [-d=<path> | -r=<owner/name>]
     <GLOB>

flox [<general-options>] sandbox revoke
     [-d=<path> | -r=<owner/name>]
     <GLOB>
```

# DESCRIPTION

`flox sandbox` is the human-facing front end to the sandbox broker that runs
inside an activation started with `flox activate --sandbox ask` (see
[`flox-activate(1)`](./flox-activate.md)).

Under `ask`, an out-of-policy file access is denied and queued rather than
silently allowed or blocked. `flox sandbox` reviews that queue, approves or
denies requests, and inspects the saved grant set.

This command is an experimental prototype gated behind the `sandbox_activate`
feature flag; set `FLOX_FEATURES_SANDBOX_ACTIVATE=true` to use it.

## Live approval

A grant approved while an `ask` session is active takes effect immediately in
that session. The denied operation can be retried and will succeed once the
short negative cache expires — no re-activation is needed. This is the core
loop: a tool's read is denied and queued in one terminal; the grant is
approved in another with `flox sandbox`; the tool retries and succeeds.

## Persistence

An "allow always" grant is written to `grants.toml` under the environment's
`.flox/cache/sandbox/` directory. At the next activation those grants are
folded into the sandbox allow-set, so a path approved once is not asked about
again. Grants are op-recorded but op-blind in enforcement: a saved grant allows
all access kinds on its paths in later sessions.

## Self-approval guard

The approval verbs (`allow`, `revoke`) are refused when run from inside the
sandboxed session they would modify. The check is enforced twice: once by this
command (via the session env marker) and again by the broker (via the
connecting process's credentials, which an environment-variable change cannot
evade). A coding agent running inside the sandbox therefore cannot approve its
own pending requests.

This is friction plus audit, not containment. Every grant is journaled, and
`flox sandbox list` (and the activation banner) flag any grant present in
`grants.toml` but missing from the journal as possibly self-approved.

# SUBCOMMANDS

`flox sandbox`
:   With no subcommand, print a status summary and, on a terminal with pending
    requests, an interactive review. Each request offers: allow this file for
    the session, allow this file always, allow everything in the parent
    directory always (only when the path is not sensitive), deny for the
    session, or decide later. Pressing Esc keeps a request queued.

`flox sandbox list`
:   List saved grants (pattern, ops, source, date, evidence), the current
    session grants, the sensitive set that is never auto-granted, and the
    allow-set cap consumption.

`flox sandbox allow <GLOB>`
:   Allow a path glob without prompting and save it to `grants.toml`. When a
    session is active the grant applies immediately and clears any matching
    pending requests; otherwise the grant is written for the next activation.

`flox sandbox revoke <GLOB>`
:   Remove a saved or session grant by its exact pattern.

# SENSITIVE PATHS

Credential and secret locations are never auto-granted and are never folded
into a directory grant. The default set covers SSH keys, cloud and Kubernetes
credentials, GPG, `.netrc`, the GitHub CLI config, any `.env` file, and the
sandbox grants directory itself. Override it with `FLOX_SANDBOX_SENSITIVE` (a
space-separated list of globs).

```{.include}
./include/environment-options.md
./include/general-options.md
```

# EXAMPLES

Approve a queued read from a second terminal, then retry in the first:
```console
$ flox sandbox allow '~/.config/gh/hosts.yml'
✅ Saved grant '~/.config/gh/hosts.yml' (cleared 1 pending) — future sessions won't ask.
```

Allow a whole directory subtree at once:
```console
$ flox sandbox allow '~/.cargo/registry/**'
✅ Saved grant '~/.cargo/registry/**' — future sessions won't ask.
```

Inspect saved grants and cap consumption:
```console
$ flox sandbox list
Saved grants for environment 'myproject'
  ...
```

Remove a grant:
```console
$ flox sandbox revoke '~/.cargo/registry/**'
✅ Removed '~/.cargo/registry/**' from grants.toml.
```

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md)
