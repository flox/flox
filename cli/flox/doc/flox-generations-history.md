---
title: FLOX-GENERATIONS-HISTORY
section: 1
header: "Flox User Manuals"
...

# NAME

flox-generations-history - list generations history of the environment

# SYNOPSIS

```
flox [<general-options>] generations history
     [-d=<path> | -r=<owner/name>]
```

# DESCRIPTION

List generations history of the environment.

For environments pushed to FloxHub, every modification to the environment
creates a new generation of the environment.

`flox generations history` prints all generation changes of the environment,
including the creation of new generations and changes between existing
generations.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`flox-generations-list(1)`](./flox-generations-list.md)
[`flox-generations-rollback(1)`](./flox-generations-rollback.md)
[`flox-generations-switch(1)`](./flox-generations-switch.md)
