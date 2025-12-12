---
title: FLOX-PULL
section: 1
header: "Flox User Manuals"
...

# NAME

flox-pull - pull environment from FloxHub

# SYNOPSIS

```
# Pull a new environment into a directory
flox [<general-options>] pull <owner>/<name>
     [-d=<path>]
     [-f]
     [-c]
     [-g=<generation>]

# Update an existing environment in a directory
flox [<general-options>] pull
     [-d=<path>]
     [-f]

# Fetch updates for a remote environment
flox [<general-options>] pull -r <owner>/<name>
     [-f]
```

# DESCRIPTION

Pull an environment from FloxHub and create a local reference to it,
or, if an environment has already been pulled, retrieve any updates.

## Pulling a new environment (`<owner>/<name> [--dir <dir>]`)

Create an environment in the current directory or the directory specified by
the `--dir` flag, that is linked to the centrally managed environemnt
`<owner>/<name>` on FloxHub.
You can make changes locally and push them back with
[`flox-push(1)`](./flox-push.md).

Alternatively, the `--copy` flag allows you to create an environment,
but does not link it to its upstream on FloxHub.
Optionally, the `--generation <generation>` can be used to select a specific
generation to create a copy of.

## Updating an existing environment in a directory (`[--dir]`)

Without a `<owner>/<name>` argument, updates an environment that has already
been pulled into the current directory, or the directory specified by the
`--dir` flag .

`-f` may be specified to forcibly update the environment locally even if
there are local changes not reflected in the remote environment.

## Updating FloxHub environments (`--reference <owner>/<name>`)

When using the `--reference` flag, commands will operate on a
copy of the environment stored in Flox's cache directory.
Any changes made to an environment using the `--reference` flag,
affect only the local copy and must be explicitly updated on FloxHub
using [`flox-push(1)`](./flox-push.md).

`flox pull --reference <owner>/<name>` will create such a local copy for the
specified environment, or update an existing copy.
This allows you to work offline with cached environments and only sync when
you choose to.

## Platform Support

A remote environment may not support the architecture or operating system of the
local system pulling the environment,
in which case `-f` may be passed to forcibly add the current system to the
environment's manifest.
This may create a broken environment that cannot be pushed back to FloxHub until
it is repaired with [`flox-edit(1)`](./flox-edit.md).
See [`manifest.toml(5)`](./manifest.toml.md) for more on multi-system
environments.

# OPTIONS

## Pull Options

`-d`, `--dir`
:   Directory to pull an environment into, or directory that contains an
    environment that has already been pulled (default: current directory).

    Cannot be used with `--reference`.

`<owner>/<name>`
:   ID of the environment to pull into a directory.

    This is used when pulling a new environment for the first time.

`-f`, `--force`
:   Forcefully overwrite the local copy of the environment,
    and accept any kind of modification and possibly incompatible results
    that have to be addressed manually.

`-c`, `--copy`
:   Create a local copy of an environment by removing the connection to the
    upstream environment on FloxHub.
    When pulling a new environment this creates a new environment
    that can be used locally or pushed to FloxHub under a new user or name.

`-g <generation>`, `--generation <generation>`
:   Pull the specified generation instead of the live generation.
    Must be used with `--copy`.

`-r <owner>/<name>`, `--reference <owner>/<name>`
:   Pull updates for a cached remote environment by reference.

    This updates a remote environment that has been activated or pulled
    locally and is cached in `~/.cache/flox/remote/`.

    Cannot be used with `--dir`, `--copy`, or `--generation`.

```{.include}
./include/general-options.md
```

# SEE ALSO

[`flox-push(1)`](./flox-push.md)
[`flox-edit(1)`](./flox-edit.md)
[`manifest.toml(5)`](./manifest.toml.md)
