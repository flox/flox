---
title: FLOX-GENERATIONS-LIST
section: 1
header: "Flox User Manuals"
...

# NAME

flox-generations-list - show all environment generations that you can switch to

# SYNOPSIS

```
flox [<general-options>] generations list
     [-d=<path> | -r=<owner/name>]
     [-u]
     [-t | --json]
     [--no-pager]
```

# DESCRIPTION

Show all environment generations that you can switch to.

For environments pushed to FloxHub, every modification to the environment
creates a new generation of the environment.

`flox generations list` prints all generations of the environment, including
which generation is currently live.

# OPTIONS

`--tree`, `-t`
:   Render generations as a tree

`--json`
:   Render generations as json (mutually exclusive with `-t`)
    Attention: the output is not guaranteed to be stable.

`--no-pager`
:   Explicitly disable paged output

```{.include}
./include/environment-options.md
./include/upstream-option.md
./include/general-options.md
```

# SEE ALSO
[`flox-generations-history(1)`](./flox-generations-history.md)
[`flox-generations-rollback(1)`](./flox-generations-rollback.md)
[`flox-generations-switch(1)`](./flox-generations-switch.md)
