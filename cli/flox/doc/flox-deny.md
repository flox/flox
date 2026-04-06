---
title: FLOX-DENY
section: 1
header: "Flox User Manuals"
...


# NAME

flox-deny - deny auto-activation for an environment

# SYNOPSIS

```
flox [<general options>] deny
     [--path=<path>]
```

# DESCRIPTION

Denies auto-activation for the environment in the current directory,
or for the environment at the path specified by `--path`.

When auto-activation is denied for an environment, `flox hook-env`
will not automatically activate the environment when the user enters
a directory containing it. Instead, the user will see a notice
suggesting `flox allow` to re-enable auto-activation.

To allow auto-activation, use `flox allow`.

# OPTIONS

## Deny Options

`--path`
:   Path to the .flox directory to deny (defaults to current directory).

# SEE ALSO

*flox-allow*(1),
*flox-activate*(1)
