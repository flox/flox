---
title: FLOX-ACTIVATE
section: 1
header: "Flox User Manuals"
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
    Spawns the command in a subshell
    that does not leak into the calling process.


# ENVIRONMENT VARIABLES

`$FLOX_ENV`
:   Absolute path to the _install prefix_ of the environment being activated.

    Set by `flox activate` before executing `shell.hook`.

    If multiple environments are indicated, each `shell.hook` will run with
    its associated `FLOX_ENV` set properly, and the activated environment
    will have `FLOX_ENV` set to the first environment indicated on the CLI.

    This variable may be used to set other environment variables such as
    `MANPATH`, `PKG_CONFIG_PATH`, `PYTHONPATH`, etc so that relevant tooling
    will search these directories to locate files and resources from
    the environment.

    **N.B.** the default shell hook for newly-created environments will
    source the `$FLOX_ENV/etc/profile` file at activation if it exists.
    This behavior can be viewed/modified with `flox edit`.

## Language packs _(**experimental**)_

Language packs help you develop with flox the way **you** work, making it
possible to install and use compilers, interpreters, libraries and modules
in much the way you would on any other operating system.

Language packs are activated by way of `$FLOX_ENV/etc/profile` as described
above, and the `flox.etc-profiles` package provides a version of this script
along with "language packs" providing environment variables and hooks that
support developing in a variety of languages.

Install a bundle of all language packs with the command:

```
flox install flox.etc-profiles
```

To restrict the installation to individual language packs, invoke `flox edit`
and update the installation stanza as follows:

```
packages.flox.etc-profiles = {
  meta.outputsToInstall = [ "base" "common_paths" "python3" ];
};
```

Please note that the `base` and `common_paths` language packs are required
when installing individual language packs.

# EXAMPLES:

-   activate "default" flox environment only within the current shell
    (add to the relevant "rc" file, e.g. `~/.bashrc` or `~/.zprofile`)

    ```
    eval "$(flox activate)"
    ```

-   activate "foo" and "default" flox environments in a new subshell

    ```
    flox activate -e foo
    ```

-   invoke command using "foo" and "default" flox environments

    ```
    flox activate -e foo -- cmd --cmdflag cmdflagarg cmdarg
    ```
