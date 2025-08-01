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
     [-s]
     [-m=(dev|run)]
     [-- <command> [<arguments>]]
```

# DESCRIPTION

Configures a shell with everything defined by the environment:

* Downloads packages and adds their `bin` directories to your `$PATH`.
* Sets environment variables and aliases.
* Runs hooks.
* Starts services (if `--start-services` is specified).

`flox activate` may run in one of three modes:

* interactive: `flox activate` when invoked from an interactive shell\
  Launches an interactive sub-shell.
  The shell to be launched is determined by `$FLOX_SHELL` or `$SHELL`.
* command: `flox activate -- CMD`\
  Executes `CMD` in the same environment as if run inside an interactive shell
  produced by an interactive `flox activate`
  The shell `CMD` is run by is determined by `$FLOX_SHELL` or `$SHELL`.
* in-place: `flox activate` when invoked from a non-interactive shell
  with it's `stdout` redirected e.g. `eval "$(flox activate)"`\
  Produces commands to be sourced by the parent shell.
  Flox will determine the parent shell from `$FLOX_SHELL` or otherwise
  automatically determine the parent shell and fall back to `$SHELL`.

`flox activate` currently supports `bash`, `fish`, `tcsh`, and `zsh` shells
for any of the detection mechanisms described above.

When invoked interactively,
the shell prompt will be modified to display the active environments,
as shown below:
```
flox [env1 env2 env3] <normal prompt>
```

When multiple environments are activated each of their shell hooks
(`profile` and `hook` scripts)
are executed in the context of the environment that they come from.
This means that for each shell hook various environment variables such as
`PATH`, `MANPATH`, `PKG_CONFIG_PATH`, `PYTHONPATH`, etc,
are set to the appropriate values for the environment in which the shell
hook was defined.
See [`manifest.toml(5)`](./manifest.toml.md) for more details on shell hooks.

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
    Environments owned by the current user and Flox are always trusted.
    You may set certain environments to always be trusted using the config key
    `trusted_environments."<owner/name>" = (trust | deny)`,
    or via the following command:
    `flox config --set trusted_environments.\"<owner/name>\" trust`.

`--print-script`
:  Prints an activation script to `stdout` that's suitable for sourcing in
   a shell rather than activation via creating a subshell.
   `flox` automatically knows when to print the activation script to `stdout`,
   so this command is just a debugging aid for users.

`-s`, `--start-services`
:  Start the services listed in the manifest when activating the environment.
   If no services are running, the services from the manifest will be started,
   otherwise a warning will displayed and activation will continue.

   This flag is currently incompatible with "in-place" activations,
   but this feature will be added in the future.

   The services started with this flag will be cleaned up once the last
   activation of this environment terminates.

   A remote environment can only have a single set of running services,
   regardless of how many times the environment is activated concurrently.

`-m (dev|run)`, `--mode (dev|run)`
:  Activate the environment in either "dev" or "run" mode.
   Overrides the `options.activate.mode` setting in the manifest.
   See [`manifest.toml(5)`](./manifest.toml.md) for more details on activation
   modes.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# ENVIRONMENT VARIABLES

## Variables set by `flox activate`

`$FLOX_ENV`
:   Contains the path to the built environment. This directory contains a merged
    set of `bin`, `lib`, etc directories for all the packages in the
    environment.

`$FLOX_PROMPT_ENVIRONMENTS`
:   Contains a space-delimited list of the active environments,
    e.g. `owner1/foo owner2/bar local_env`.
    If, `hide_default_prompt` is set to `true`, environments named `default` are
    excluded.

`$FLOX_ENV_CACHE`
:   `activate` sets this variable to a directory that can be used by an
    environment's hook to store transient files.
    These files will persist for environments used locally,
    but they will not be pushed,
    and they will not persist when using a remote environment with `-r`.

`$FLOX_ENV_PROJECT`
:   `activate` sets this variable to the directory of the project using the Flox
    environment.
    For environments stored locally, this is the directory containing the
    environment.
    When running `flox activate -r`, this is set to the current working
    directory.
    This variable can be used to find project files in environment hooks.

`$FLOX_ENV_DESCRIPTION`
:  `activate` sets this variable to the project name of the environment. It can
    be used to identify or construct messages about the environment.

`$_FLOX_ACTIVE_ENVIRONMENTS`
:   A JSON array containing one object per active environment.
    This is currently an implementation detail
    and its contents are subject to change.

`$FLOX_ACTIVATE_START_SERVICES`
:   `"true"` if this activation started services, `"false"` otherwise.

## Variables used by `flox activate`

`$FLOX_SHELL`, `$SHELL`
:  When activating an environment
   Flox will either launch a sub-shell
   or emit commands to configure an already-running (parent) shell.
   In both of these cases Flox needs to know which shell to use,
   and these variables are used to control the selection process.

       * interactive and command modes: When launching a sub-shell
         Flox will invoke
         the shell specified in `$FLOX_SHELL` if set
         or fall back to invoke `$SHELL` by default.

       * in-place mode: When performing an "in place" activation
         Flox will attempt to detect its parent shell type unless overridden by
         the `$FLOX_SHELL` variable,
         and if it cannot detect its parent shell type then will
         produce a script with syntax determined by `$SHELL`.

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
