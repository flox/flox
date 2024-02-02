---
title: FLOX-ACTIVATE
section: 1
header: "flox User Manuals"
...

# NAME

flox-activate - activate environments

# SYNOPSIS

```
flox [ <general-options> ] activate
     [-d=<path> | -r=<owner/name>]
     [ -t ]
     [ -- <command> [ <arguments> ] ]
```

# DESCRIPTION

Sets environment variables and aliases,
runs hooks,
and adds `bin` directories to your `$PATH`.

Launches an interactive subshell when invoked as `flox activate` from an
interactive shell.
Launches a subshell non-interactively when invoked with a command and
arguments.
May also be invoked as `$(flox activate)` to produce commands to be sourced
by your current `$SHELL`.

When invoked interactively,
the shell prompt will be modified to display the active environments,
as shown below:
```
flox [env1 env2 env3] <normal prompt>
```

# OPTIONS

## Activate Options

`-- <command> [ <arguments> ]`
:   Command to run in the environment.
    Spawns the command in a subshell
    that does not leak into the calling process.

`-t`, `--trust`
:   Trust a remote environment for this activation.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# ENVIRONMENT VARIABLES

## `$FLOX_ENV`
The absolute path to the _install prefix_ of the environment being activated.
Set by `flox activate` before executing `shell.hook`.

If multiple environments are active,
each `shell.hook` will run with its associated `FLOX_ENV` set properly,
and the activated environment will have `FLOX_ENV` set to the left-most
environment listed in the prompt.
This variable may be used to set other environment variables such as `MANPATH`,
`PKG_CONFIG_PATH`,
`PYTHONPATH`,
etc so that relevant tooling will search these directories to locate files and
resources from the environment.

**N.B.** the default shell hook for newly-created environments will
source the `$FLOX_ENV/etc/profile` file at activation if it exists.
This behavior can be viewed and/or modified with `flox edit`.

## `$FLOX_ACTIVE_ENVIRONMENTS`
A JSON array containing one object per active environment.
This is currently an implementation detail and its contents are subject to
change.

# EXAMPLES:

Activate an environment stored in the current directory:

```
$ flox activate
```

Activate an environment `some_user/myenv` that's been pushed to FloxHub:

```
$ flox activate -r some_user/myenv
```

Invoke a command inside an environment without entering its subshell:

```
$ flox activate -- cmd --some-arg arg1 arg2
```

Activate `default` flox environment only within the current shell
(add to the relevant "rc" file, e.g. `~/.bashrc` or `~/.zprofile`):

```
$ eval "$(flox activate)"
```

# See also
[`flox-push(1)`](./flox-push.md),
[`flox-pull(1)`](./flox-pull.md)
