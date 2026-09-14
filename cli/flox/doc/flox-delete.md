---
title: FLOX-DELETE
section: 1
header: "Flox User Manuals"
...


# NAME

flox-delete - delete an environment

# SYNOPSIS

```text
# Delete an environment in a directory
flox [<general options>] delete
     [-f]
     [-d=<path>]

# Delete the local copy of a FloxHub environment
flox [<general options>] delete
     [-f]
     [-r=<owner/name> | -D]
```

# DESCRIPTION

Deletes all data pertaining to an environment.
By default, only the environment in the current directory is deleted,
but environments in other directories may be deleted via the `-d` flag.

The `--reference` and `--default` flags instead delete the local copy of a
FloxHub environment that was cached on this machine by a command run with
`--reference`, such as [`flox-activate(1)`](./flox-activate.md) or
[`flox-pull(1)`](./flox-pull.md).
Only the local copy is removed:
the environment on FloxHub is *not* deleted,
and will be downloaded again the next time you activate or pull the reference.
Deleting the upstream environment on FloxHub is not currently supported from
the command line.
This is a local operation and does not require network access, so it can also
clean up a cached copy that can no longer be activated.

By default, you will be prompted for a confirmation before deleting the
environment.
The `-f` flag skips the confirmation dialog,
and is required to delete the local copy of a FloxHub environment in a
non-interactive context.

# OPTIONS

## Delete Options

`-f`, `--force`
:   Delete the environment without confirmation.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`flox-init(1)`](./flox-init.md),
[`flox-push(1)`](./flox-push.md),
[`flox-pull(1)`](./flox-pull.md)
