---
title: FLOX-ACTIVATE
section: 1
header: "flox User Manuals"
...

# NAME

flox-activate - activate environments

# SYNOPSIS

flox [ `<general-options>` ] activate [ `<options>` ] [ -- `<command>` [ `<argument>` ] ]

# DESCRIPTION

Sets environment variables and aliases, runs hooks and adds environment
`bin` directories to your `$PATH`. Can be invoked from an interactive
terminal to launch a sub-shell, non-interactively to produce
a series of commands to be sourced by your current `$SHELL`,
or with a command and arguments to be invoked directly.



# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```

## Activate Options

[ -- `<command>` [ `<argument>` ] ]
:   Command to run in the environment.
    Spawns the command in an ephmenral environemnt
    that does not leak into the calling process.


# EXAMPLES:

-   activate "default" flox environment only within the current shell
    (add to the relevant "rc" file, e.g. `~/.bashrc` or `~/.zprofile`)

    ```
    . <(flox activate)
    ```

-   activate "foo" and "default" flox environments in a new subshell

    ```
    flox activate -e foo
    ```

-   invoke command using "foo" and "default" flox environments

    ```
    flox activate -e foo -- cmd --cmdflag cmdflagarg cmdarg
    ```
