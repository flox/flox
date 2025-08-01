---
title: FLOX-GENERATIONS-ROLLBACK
section: 1
header: "Flox User Manuals"
...

# NAME

flox-generations-rollback - switch to the previous generation

# SYNOPSIS

```
flox [<general-options>] generations rollback
     [-d=<path> | -r=<owner/name>]
```

# DESCRIPTION

Switch to the previous generation of the environment.

Rolling back to the previous generation restores the environment's manifest and
lockfile to the state of the previous generation.

It sets the previous generation as the current generation,
and it adds an entry to the history of generations.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`flox-generations-history(1)`](./flox-generations-history.md)
[`flox-generations-list(1)`](./flox-generations-list.md)
[`flox-generations-switch(1)`](./flox-generations-switch.md)

