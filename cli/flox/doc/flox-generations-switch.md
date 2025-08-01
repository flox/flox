---
title: FLOX-GENERATIONS-SWITCH
section: 1
header: "Flox User Manuals"
...

# NAME

flox-generations-switch - switch to the provided generation

# SYNOPSIS

```
flox [<general-options>] generations switch
     [-d=<path> | -r=<owner/name>]
     --target-generation=<generation>
```

# DESCRIPTION

Switch to the provided generation of the environment.

Switching generation restores the environment's manifest and lockfile to the
state of the specified generation.

It sets the specified generation as the current generation,
and it adds an entry to the history of generations.

# OPTIONS

`--target-generation <number>`
:   What generation number to switch to.
    Generation numbers can be found with
    [`flox-generations-list(1)`](./flox-generations-list.md).

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`flox-generations-history(1)`](./flox-generations-history.md)
[`flox-generations-list(1)`](./flox-generations-list.md)
[`flox-generations-rollback(1)`](./flox-generations-rollback.md)

