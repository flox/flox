---
title: FLOX-PUSH
section: 1
header: "Flox User Manuals"
...


# NAME

flox-push - send environment to FloxHub

# SYNOPSIS

```
flox [<general-options>] push
     [-d=<path>]
     [-o=<owner>]
     [-f]

flox [<general-options>] push
     -r=<owner>/<name>
     [-f]
```

# DESCRIPTION

Push an environment to FloxHub to share and centrally manage it with flox,
or sync changes made locally to a remote environment.

After pushing, the remote environment can be referred to as `<owner>/<name>`.

## Pushing from a directory (using `--dir | -d`)

A path environment contains a manifest file and lock file which are stored
locally and possibly committed to version control.
Pushing the environment moves the manifest and lock file to FloxHub,
leaving a reference to the revision of the environment stored locally.

Once the environment has been pushed, it can be used directly with the
`--remote` option,
or it can be used and edited locally before syncing with `flox push`.
See [`flox-edit(1)`](./flox-edit.md), [`flox-install(1)`](./flox-install.md),
and [`flox-uninstall(1)`](./flox-uninstall.md) for editing the environment.


## Pushing a remote environment (using `--remote | -r`)

When using the `--remote` flag, commands will operate on a
**central persistent local copy** of the environment.
Any changes made to an environment using the `--remote` flag,
affect only the local copy and must be explicitly updated on FloxHub
using `flox push --remote`

## Conflict resolution

In the same way as a git repo, local changes to an environment that has been
pushed may diverge from the environment on FloxHub if `flox push` is run from a
different host.
Passing `--force` to `flox push` will cause it to overwrite any changes on
FloxHub with local changes to the environment.

# OPTIONS

## Push Options

`-d`, `--dir`
:   Directory to push the environment from (default: current directory).

    Cannot be used with `--remote`.

`-o`, `--owner`, `--org`
:   FloxHub owner to push environment to (default: current FloxHub user).

    Can only be specified when pushing an environment for the first time.
    Use 'flox pull --copy' to copy an existing environment and push it to a new
    owner.

    Cannot be used with `--remote`.


`-r`, `--remote`
:   Update a remote environment by reference (e.g., `owner/name`).

    This pushes the local changes made to a remote environment
    using commands with the `--remote` flag.
    Referring to environments, that have never been accessed
    or explicitly pulled will cause an error.

    Cannot be used with `--dir` or `--owner`.


`-f`, `--force`
:   Forcibly overwrite the remote copy of the environment.

```{.include}
./include/general-options.md
```

# SEE ALSO

[`flox-pull(1)`](./flox-pull.md)
