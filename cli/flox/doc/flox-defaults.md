---
title: FLOX-DEFAULTS
section: 1
header: "Flox User Manuals"
...

# NAME

flox-defaults - save and use a startup environment

# SYNOPSIS

```text
flox [<general-options>] defaults set [--json] (<owner>/<name> | --dir <path>)
flox [<general-options>] defaults list [--json]
flox [<general-options>] defaults clear [--json]
flox [<general-options>] defaults activate [<activation-options>]
flox [<general-options>] defaults setup --shell <shell> --print
```

# DESCRIPTION

Save zero or one environment to activate when an interactive shell starts.
Install the integration printed by `defaults setup` manually to enable startup
activation. Saving a selection alone does not install shell integration or
change any running shell.

The saved selection is independent of the existing user default environment
selected by `flox activate -D` or `--default`.
Plain `flox activate` keeps its ordinary environment selection behavior.

## set

Save one full published reference, such as `acme/fred`, or an explicit local
directory containing `.flox`, such as `--dir /path/to/project`.
The local directory is saved as a canonical absolute path, so environments
with the same name remain distinct and symbolic links resolve to the same
target. A local target must have environment metadata when saved.

Saving does not authenticate, activate, build, or copy an environment.
A published reference is saved without checking source availability.
Saving the same selector again makes no change.
Multiple selectors and combinations of a reference and `--dir` are errors.

## list

Show the saved selector. This is configuration, not a claim about the current
shell's active environment; use `flox envs --active` to inspect active state.
Manual integration is reported as `unverified`, because this command does not
inspect or execute shell startup files.
Listing does not require login or access to the saved source.

## clear

Remove the saved selection. Future eligible shell startups do nothing.
Running shells, source environments, and installed manual integration remain
unchanged. Clearing does not require login or access to the saved source.

## activate

Activate the saved target using ordinary
[`flox-activate(1)`](./flox-activate.md) behavior and options:
`--trust`, `--print-script`, `--start-services`, `--no-start-services`,
`--add-sbin`, `--mode`, `--generation`, `-c`, and `-- <command> [<args>]...`.
Invocation detection, hooks, profiles, services, trust checks, and layering
follow ordinary activation. This command does not replace the existing stack
or change the saved selector.

Local directory names containing quotation marks can fail in ordinary
activation and therefore in defaults activation or startup. Saving and listing
the canonical path still work. This existing limitation is tracked separately
in [CLI-256](https://linear.app/floxdotdev/issue/CLI-256/optional-handle-quotation-marks-in-activation-directory-names).

An interactive invocation opens a child shell. Evaluated activation applies
to the evaluating shell. `-c` runs a shell command, while `--` executes a
command directly. With no saved target, activation returns an actionable
error; it does not fall back to the current directory or user default.

## setup

Print instructions and a complete dynamic startup block for `bash`, `zsh`,
`fish`, or `tcsh`. Both `--shell` and `--print` are required.
This command writes no configuration or startup files.
Review the output and add the block to the specified file once, after Flox
has been made available on `PATH`. Existing startup lines continue to run;
review any earlier activation lines when installing the block.

The block reads the saved selector at each eligible interactive startup and
uses in-place activation, so startup does not open another interactive child.
It contains no fixed environment reference or local path.

For example, save `acme/fred` and install the printed block. Independent shells
then start with Fred. Saving `acme/bob` affects subsequent eligible shells;
existing shells stay in Fred. A nested shell inheriting an active Flox stack
preserves that stack and skips startup selection. After ordinary deactivation
actually leaves a parent inactive, a new interactive child can load Bob.

This guard uses inherited Flox active state. A terminal server such as tmux
may supply its stored environment, so new windows follow that supplied state.
An absent or empty active stack is eligible. Malformed inherited active state
produces a warning and skips startup activation.

With no selection, startup does nothing. A missing source or invalid saved
configuration is an error and leaves the saved selector intact. The loader
discards generated output if the activation command fails, reports errors,
and lets shell startup continue. It also checks the result of evaluating the
generated script and reports possible partial activation effects when a failure
status returns to the loader. Some fatal tcsh parse errors bypass that warning.
Ordinary activation scripts can mask failures inside hooks or profiles; a
successful final script status does not guarantee every inner command
succeeded. There is no rollback or automatic retry loop.

## JSON output

`set`, `list`, and `clear` support `--json`.
The inventory contains a nullable `selection` and conservative integration
status. Mutation responses also include `changed`:

```json
{
  "changed": true,
  "integration": {"status": "unverified"},
  "selection": {"kind": "remote", "reference": "acme/fred"}
}
```

A local selection has `{"kind":"local","path":"/absolute/project"}`.
Cleared or absent selection is `null`.
Selection and configuration errors produce a JSON `error` object with `code`
and `message`, a diagnostic on standard error, and a nonzero exit status.
Command-line syntax errors follow ordinary CLI error conventions.

# FILES

`defaults.toml` in the resolved Flox configuration directory
:   Versioned selector configuration. The directory follows Flox configuration
    conventions, including `FLOX_CONFIG_DIR`. Mutations use a sibling
    `defaults.lock` to serialize writers and atomically replace the document.
    Invalid configuration is reported instead of silently overwritten.

# EXAMPLES

```console
$ flox defaults set acme/fred
$ flox defaults setup --shell bash --print
```

Review and install the printed block in your Bash startup file, then open a
new interactive shell.

```console
$ flox defaults set --dir '/path/to/local project'
$ flox defaults list --json
$ flox defaults activate --mode run -- command arg
$ flox defaults clear --json
```

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md),
[`flox-deactivate(1)`](./flox-deactivate.md),
[`flox-envs(1)`](./flox-envs.md),
[`flox-config(1)`](./flox-config.md)
