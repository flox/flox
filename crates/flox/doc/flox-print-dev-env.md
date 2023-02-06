---
title: FLOX-PRINT_DEV_ENV
section: 1
header: "flox User Manuals"
...


# NAME

flox-print-dev-env - print shell code that can be sourced by bash
                     to reproduce the development environment

# SYNOPSIS

flox [ `<general-options>` ] print-dev-env [ `<options>` ] `<package>`

# DESCRIPTION

Print a shell script that can be sourced by the current shell
to both activate a project flox environment (if it exists)
and the build environment for the corresponding package.
This allows you to enter a development environment in
your current shell rather than in a subshell (as with flox develop).

# OPTIONS

```{.include}
./include/general-options.md
./include/development-options.md
```

`<package>`
:   the package or project flox environment to prit the environment script for
