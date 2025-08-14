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
     <generation>
```

# DESCRIPTION

Switch to the provided generation of the environment.

Generation numbers can be found with
[`flox-generations-history(1)`](./flox-generations-history.md) or
[`flox-generations-list(1)`](./flox-generations-list.md).

Switching generation restores the environment's manifest and lockfile to the
state of the specified generation, sets it as the live generation, and adds
an entry to generation history.

Generations don't always have a linear history. If you create generation 2 by
installing a package, rollback to generation 1 and create generation 3 by
installing another package, then generation 3 won't contain the package from
generation 2.

[`flox-generations-history(1)`](./flox-generations-history.md) can be used to
see the relationships between generations.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`flox-generations-history(1)`](./flox-generations-history.md)
[`flox-generations-list(1)`](./flox-generations-list.md)
[`flox-generations-rollback(1)`](./flox-generations-rollback.md)

