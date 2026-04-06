---
title: FLOX-ALLOW
section: 1
header: "Flox User Manuals"
...


# NAME

flox-allow - allow auto-activation for an environment

# SYNOPSIS

```
flox [<general options>] allow
     [--path=<path>]
```

# DESCRIPTION

Allows auto-activation for the environment in the current directory,
or for the environment at the path specified by `--path`.

When auto-activation is allowed for an environment, `flox hook-env`
will automatically activate the environment when the user enters
a directory containing it.

For local (path-based) environments, allowing auto-activation also
implicitly trusts the environment.

To deny auto-activation, use `flox deny`.

# OPTIONS

## Allow Options

`--path`
:   Path to the .flox directory to allow (defaults to current directory).

# SEE ALSO

*flox-deny*(1),
*flox-activate*(1)
