---
title: FLOX-EDIT
section: 1
header: "Flox User Manuals"
...


# NAME

flox-edit - edit declarative environment configuration

# SYNOPSIS

```
flox [<general options>] edit
     [-d=<path> | -r=<owner/name>]
     [[-f=<file>] | -n=<name>]
```

# DESCRIPTION

Transactionally edit the environment manifest.
By default invokes an editor with a copy of the local manifest for the user to
interactively edit.
The editor is found by querying `$EDITOR`, `$VISUAL`,
and then by looking in `$PATH` for a list of common editors.
An environment's manifest that exists on FloxHub or in a different directory
can be edited via the `-r` a `-d` flags respectively.

Once the editor is closed the environment is built in order to validate the
edit.
The edit is discarded if the build fails.
This transactional editing prevents an edit from leaving the environment in a
broken state.

The environment can be edited non-interactively via the `-f` flag,
which replaces the contents of the manifest with those of the provided file.

# OPTIONS

## Edit Options

`-f`, `--file`
:   Replace environment manifest with that in `<file>`.

`-n`, `--name`
:   Rename the environment to `<name>`.

[ (\--file|-f) `<file>` ]
:   Replace environment declaration with that in `<file>`.
    If `<file>` is `-`, reads from stdin.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`flox-push(1)`](./flox-push.md),
[`flox-pull(1)`](./flox-pull.md),
[`flox-activate(1)`](./flox-activate.md)
