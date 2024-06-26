---
title: FLOX-EDIT
section: 1
header: "Flox User Manuals"
...


# NAME

flox-edit - edit the declarative environment configuration

# SYNOPSIS

```
flox [<general options>] edit
     [-d=<path> | -r=<owner/name>]
     [[-f=<file>] | -n=<name> | --sync | --reset]
```

# DESCRIPTION

## Transactionally edit the environment manifest.

By default invokes an editor with a copy of the local manifest for the user to
interactively edit.
The editor is found by querying `$EDITOR`, `$VISUAL`,
and then by looking for common editors in `$PATH`.
The manifest of an environment on FloxHub or in a different directory
can be edited via the `-r` or `-d` flags respectively.
See [`manifest.toml(5)`](./manifest.toml.md) for more details on the manifest
format.

Once the editor is closed the environment is built in order to validate the
edit.
If the build fails you are given a change to continue editing the manifest,
and if you decline, the edit is discarded.
This transactional editing prevents an edit from leaving the environment in a
broken state.
One exception is the `-n` flag,
which renames a local environment but does not rebuild it.

The environment can be edited non-interactively via the `-f` flag,
which replaces the contents of the manifest with those of the provided file.

## Sync the local manifest with the current generation.

When unsing environments that were pushed or pulled from FloxHub,
local changes to the manifest in `.flox/env/manifest.toml`
will block the use of imperative environment commands
`flox {install, uninstall, upgrade}`.
In this case, a new generation has to be created from the local changes first
or the local changes discarded.
Run `flox edit --sync` to create a new generation,
or `flox edit --reset` to discard local changes
and reset to the current latest generation.


# OPTIONS

## Edit Options

`-f`, `--file`
:   Replace environment manifest with that in `<file>`.
    If `<file>` is `-`, reads from stdin.

`-n`, `--name`
:   Rename the environment to `<name>`.
    Only works for local environments.

`-s`, `--sync`
:   Create a new generation from the current local environment
    (Only available for managed environments)

`-r`, `--reset`
:   Reset the environment to the current generation
    (Only available for managed environments)

```{.include}
./include/environment-options.md
./include/general-options.md
```

# ENVIRONMENT VARIABLES

`$EDITOR`, `$VISUAL`
:   Override the default editor used for editing environment manifests and commit messages.

# SEE ALSO
[`flox-push(1)`](./flox-push.md),
[`flox-pull(1)`](./flox-pull.md),
[`flox-activate(1)`](./flox-activate.md)
