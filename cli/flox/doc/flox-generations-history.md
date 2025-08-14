---
title: FLOX-GENERATIONS-HISTORY
section: 1
header: "Flox User Manuals"
...

# NAME

flox-generations-history - print the history of the environment

# SYNOPSIS

```
flox [<general-options>] generations history
     [-d=<path> | -r=<owner/name>]
```

# DESCRIPTION

Print the history of the environment.

For environments pushed to FloxHub, every modification to the environment
creates a new generation of the environment.
It's also possible to change the current generation by using
`flox generations switch` or `flox generations rollback`.

`flox generations history` prints what generation has been the current
generation over time.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`flox-generations-list(1)`](./flox-generations-list.md)
[`flox-generations-rollback(1)`](./flox-generations-rollback.md)
[`flox-generations-switch(1)`](./flox-generations-switch.md)
