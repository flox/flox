---
title: FLOX-GENERATIONS-ROLLBACK
section: 1
header: "Flox User Manuals"
...

# NAME

flox-generations-rollback - switch to the previous live generation

# SYNOPSIS

```
flox [<general-options>] generations rollback
     [-d=<path> | -r=<owner/name>]
```

# DESCRIPTION

Switch to the previous live generation of the environment.

Rolling back to the previous generation restores the environment's manifest and
lockfile to the state of the previous generation, sets it as the live
generation, and adds an entry to generation history.

The previously live generation isn't always N-1. If you've previously rolled
back from generation 3 -> 2 then rolling back again will take you from
generation 2 -> 3. Similarly if you've switched from generation 3 -> 1 then
rolling back will take you from generation 1 -> 3.

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
[`flox-generations-switch(1)`](./flox-generations-switch.md)

