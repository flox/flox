---
title: FLOX-ACTIVATE
section: 1
header: "Flox User Manuals"
...

# NAME

flox-activate - activate environments

# SYNOPSIS

```
flox [<general-options>] activate
     [-d=<path> | -r=<owner>/<name>]
     [-t]
     [--print-script]
     [ -- <command> [<arguments>]]
```

# DESCRIPTION

Sets environment variables and aliases,
runs hooks,
and adds `bin` directories to your `$PATH`.

Launches an interactive subshell when invoked as `flox activate` from an
interactive shell.
Launches a subshell non-interactively when invoked with a command
and arguments.
May also be invoked as `$(flox activate)` to produce commands to be sourced
by your current `$SHELL`.

When invoked interactively,
the shell prompt will be modified to display the active environments,
as shown below:
```
flox [env1 env2 env3] <normal prompt>
```

When multiple environments are activated each of their shell hooks
(`hook.script` or `hook.file`)
are executed in the context of the environment that they come from.
This means that for each shell hook various environment variables such as
`PATH`, `MANPATH`, `PKG_CONFIG_PATH`, `PYTHONPATH`, etc,
are set to the appropriate values for the environment in which the shell
hook was defined.

# OPTIONS

## Activate Options

`-- <command> [<arguments>]`
:   Command to run in the environment.
    Spawns the command in a subshell that does not leak into the calling
    process.

`-t`, `--trust`
:   Trust a remote environment for this activation.
    Activating an environment executes a shell hook which may execute arbitrary
    code.
    This presents a security risk,
    so you will be prompted whether to trust the environment.
    Environments owned by the current user are always trusted.
    You may set certain environments to always be trusted using the config key
    `trusted_environments."<owner/name>" = (trust | deny)`,
    or via the following command:
    `flox config --set trusted_environments.\"<owner/name>\" trust`.

`--print-script`
:  Prints an activation script to `stdout` that's suitable for sourcing in
   a shell rather than activation via creating a subshell.
   `flox` automatically knows when to print the activation script to `stdout`,
   so this command is just a debugging aid for users.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# ENVIRONMENT VARIABLES

`$FLOX_SHELL`
:  When launching an interactive sub-shell, Flox launches the shell specified in
   `$FLOX_SHELL` if it is set.

`$SHELL`
:  When launching an interactive sub-shell, Flox launches the shell specified in
   `$SHELL` if it is set and `$FLOX_SHELL` is not set.

`$FLOX_PROMPT_ENVIRONMENTS`
:   Contains a space-delimited list of the active environments,
    e.g. `owner1/foo owner2/bar local_env`.

`$_FLOX_ACTIVE_ENVIRONMENTS`
:   A JSON array containing one object per active environment.
    This is currently an implementation detail
    and its contents are subject to change.

`$FLOX_PROMPT_COLOR_{1,2}`
:   Flox adds text to the beginning of the shell prompt to indicate which
    environments are active.
    A set of default colors are used to color this prompt,
    but the colors may be overridden with the `$FLOX_PROMPT_COLOR_1` and
    `$FLOX_PROMPT_COLOR_2` environment variables.

    The values of these variables should be integers
    chosen from the 256-color palette as described in the
    [xterm-256color chart](https://upload.wikimedia.org/wikipedia/commons/1/15/Xterm_256color_chart.svg).

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

Activate `default` Flox environment only within the current shell
(add to the relevant "rc" file, e.g. `~/.bashrc` or `~/.zprofile`):

```
$ eval "$(flox activate)"
```

# SEE ALSO
[`flox-push(1)`](./flox-push.md),
[`flox-pull(1)`](./flox-pull.md),
[`flox-edit(1)`](./flox-edit.md),
[`flox-delete(1)`](./flox-delete.md)
