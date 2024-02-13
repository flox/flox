---
title: FLOX-DELETE
section: 1
header: "Flox User Manuals"
...


# NAME

flox-delete - delete an environment

# SYNOPSIS

```
flox [<general options>] delete
     [-f]
     [-d=<path> | -r=<owner/name>]
```

# DESCRIPTION

Deletes all data pertaining to an environment.
By default only the environment in the current directory is deleted,
but environments in other directories may be deleted via the `-d` flag.

By default you will be prompted for a confirmation before deleting the
environment.
The `-f` flag skips the confirmation dialog and is required for non-interactive
use.

# OPTIONS

## Delete Options
`-f`, `--force`
:   Delete the environment without confirmation.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# See Also
[`flox-push(1)`](./flox-push.md),
[`flox-pull(1)`](./flox-pull.md)
