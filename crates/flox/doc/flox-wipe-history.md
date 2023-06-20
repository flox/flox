---
title: FLOX-WIPE-HISTORY
section: 1
header: "flox User Manuals"
...


# NAME

flox-wipe-history - delete builds of non-current versions of an environment

flox [ `<general-options>` ] wipe-history [ `<options>` ]

# DESCRIPTION

Environment generations are composed of:
- a human editable description (which can be modified
with [`flox-edit`(1)](./flox-edit.md))
- a build of that description, which includes all the binaries that are part of
the environment

`wipe-history` cleans up old builds of an environment, but it does not delete
the description of generations, so they can still be switched to with
[`flox-rollback`(1)](./flox-rollback).
`wipe-history` always keeps the 10 most recent generations, and it only deletes
generations that have not been created or switched to for more than 90 days.
In the process, a garbage collection of the entirety of `/nix/store` is
triggered.

# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```
