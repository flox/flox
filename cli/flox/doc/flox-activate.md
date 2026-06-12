---
title: FLOX-ACTIVATE
section: 1
header: "Flox User Manuals"
...

# NAME

flox-activate - activate environments

# SYNOPSIS

```text
flox [<general-options>] activate
     [-d=<path> | -r=<owner>/<name>]
     [-t]
     [--print-script]
     [--start-services | --no-start-services]
     [-m=(dev|run)]
     [--sandbox[=(off|warn|enforce|prompt)]]
     [-g=<generation>]
     [-c=<shell command> | -- <exec command>...]
```

# DESCRIPTION

Configures a shell with everything defined by the environment:

* Downloads packages and adds their `bin` directories to your `$PATH`.
* Sets environment variables and aliases.
* Runs hooks.
* Starts services (if `--start-services` is specified).

`flox activate` may run in one of four modes:

* interactive: `flox activate` when invoked from an interactive shell\
  Launches an interactive sub-shell.
  The shell to be launched is determined by `$FLOX_SHELL` or `$SHELL`.
* shell command: `flox activate -c CMD`\
  Runs `CMD` in the same environment as if run inside an interactive shell
  produced by an interactive `flox activate`.
  The shell `CMD` is run by is determined by `$FLOX_SHELL` or `$SHELL`.
  Because `CMD` is passed to a shell, shell features like running multiple
  commands with `&&` can be used.
* exec command: `flox activate -- CMD`\
  Execs `CMD` directly after performing all parts of activation except for
  running scripts in `[profile]`.
* in-place: `flox activate` when invoked from a non-interactive shell
  with its `stdout` redirected e.g. `eval "$(flox activate)"`\
  Produces commands to be sourced by the parent shell.
  Flox will determine the parent shell from `$FLOX_SHELL` or otherwise
  automatically determine the parent shell and fall back to `$SHELL`.

`flox activate` currently supports `bash`, `fish`, `tcsh`, and `zsh` shells
for any of the detection mechanisms described above.

When invoked interactively,
the shell prompt will be modified to display the active environments,
as shown below:
```text
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

To reverse activation,
run [`flox-deactivate(1)`](./flox-deactivate.md).
Inside a `flox activate` subshell,
`flox deactivate` is equivalent to `exit`.

# AUTO-ACTIVATION

```{.include}
./include/auto-activate-experimental.md
```

Auto-activation activates an environment automatically when you enter a
directory that contains it,
and deactivates it when you leave,
so you do not have to run `flox activate` or `flox deactivate` by hand.

## Enabling auto-activation

Two things are required:

1. The Flox prompt hook must be installed in your shell.
   The hook is installed by any in-place activation,
   so add a line such as `eval "$(flox activate -D)"` to your shell's startup
   file (for example `~/.bashrc`, `~/.zshrc`, `~/.config/fish/config.fish`, or
   `~/.tcshrc`).
   If you already activate a default or other environment in-place at startup,
   the hook is already installed.
   The hook ships with Flox and stays dormant until the feature flag below is
   set.
   Run `flox config --set disable_hook true` to opt out of the hook entirely.
2. The `auto_activate` feature flag must be enabled,
   either with `FLOX_FEATURES_AUTO_ACTIVATE=true` in your environment
   or with `flox config --set features.auto_activate true`.

## How it works

On each prompt the hook looks for `.flox` environments at or above the current
directory.
An environment is auto-activated only if you have allowed it.
Allowed environments are activated outermost-first;
when you leave their directory they are deactivated again.
Services are not started unless `auto-start = true` is set in the manifest's
`[services]` section.

## Allowing and denying environments

The first time you enter a directory with an environment that you have neither
allowed nor denied,
and `auto_activate` is set to `prompt` (the default),
Flox asks before activating it:

```text
Auto-activate the environment in '/path/to/project'? [y/N]
```

Answering `y` allows the environment and activates it immediately,
and records the choice so you are not asked again.
Answering `N` (or pressing Enter) skips the environment for the current shell
session only;
you are asked again in a new shell or when you re-enter the directory.
To stop being asked for this directory, run
[`flox-activate-deny(1)`](./flox-activate-deny.md):

```bash
flox activate deny
```

Set the `auto_activate` config option to `allowed` to skip the prompt entirely
and auto-activate only environments you have already allowed.

Manage these decisions ahead of time with the
[`flox-activate-allow(1)`](./flox-activate-allow.md) and
[`flox-activate-deny(1)`](./flox-activate-deny.md) subcommands.
Decisions are stored in the user config file under `auto_activate_environments`.

See [`flox-config(1)`](./flox-config.md) for the `auto_activate`,
`auto_activate_environments`, `auto_activate_fish_mode`, and `disable_hook`
options.

# OPTIONS

## Activate Options

`-c <command>`, `--command <command>`
:   Shell command string to run in a subshell started in the activated
    environment

`-- <command> [<arguments>]`
:   Command to exec in the activated environment.
    This does not run any profile scripts

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
   otherwise a warning will be displayed and activation will continue.

   To start services by default without requiring `-s`, set
   `services.auto-start = true` in the manifest.

   The services started with this flag will be cleaned up once the last
   activation of this environment terminates.

   A remote environment can only have a single set of running services,
   regardless of how many times the environment is activated concurrently.

`--no-start-services`
:  Don't start services even if configured in the manifest with `auto-start = true`.

`-m (dev|run)`, `--mode (dev|run)`
:  Activate the environment in either "dev" or "run" mode.
   Overrides the `options.activate.mode` setting in the manifest.
   See [`manifest.toml(5)`](./manifest.toml.md) for more details on activation
   modes.

`--sandbox (off|warn|enforce|prompt)`
:  Mediate filesystem and outbound-network access during this activation.
   Defaults to `off`, which applies no sandbox.
   A bare `--sandbox` with no value is shorthand for `--sandbox prompt`.
   Environments can request a mode with the `options.sandbox` manifest
   setting (see [`manifest.toml(5)`](./manifest.toml.md)); the
   command-line flag takes precedence over the manifest.
   `warn` reports out-of-policy access without blocking it,
   `enforce` denies it, and `prompt` denies it and queues it for approval.
   File reads outside the environment closure and TCP connections to hosts
   not in the network policy are both mediated; loopback and Flox's own
   service hosts are always allowed.

   The default policy is tuned so an agent can work autonomously with
   minimal prompting:

       * The project working directory is readable and writable.
       * All `/nix/store` reads are allowed (the store holds immutable,
         world-readable packages, so tools reading their own files are not
         flagged).
       * Common non-sensitive dev configs are allowed
         (`~/.gitconfig`, `~/.npmrc`, `~/.config/{git,npm,pip}`, cargo and
         rustup config).
       * The default network allowlist covers FloxHub, the Flox Catalog,
         GitHub (clone/fetch and release downloads), the npm, PyPI, and
         crates.io registries, and Flox's own metrics endpoint (omitted
         when metrics are disabled).
       * Credential and secret files are denied even under `enforce`,
         before the `$HOME`-dotfile allowance: `~/.ssh`, `~/.aws`,
         `~/.gnupg`, `~/.kube`, `~/.netrc`, `~/.config/gh`, and any
         `.env` / `.env.*` file. Override the set with the
         `FLOX_SANDBOX_SENSITIVE` environment variable (space-separated
         globs; a leading `~/` expands to `$HOME`).
       * New files created outside an in-policy directory are denied: a
         write to a path that does not yet exist is judged by its parent
         directory's policy.

   These defaults are not invisible policy: the first sandboxed activation
   writes them to the environment's grant store as explicit `default-seed`
   grants. Inspect them with `flox sandbox list --all` and revoke any of
   them with `flox sandbox revoke` — a revoked default stays revoked. Only
   loopback and Flox's own service hosts remain hardcoded (revoking them
   would break flox itself), and the sensitive set remains a hardcoded
   denylist. Denied and warned accesses are recorded per environment and
   reviewable afterwards with `flox sandbox audit`.

   This is the `standard` profile, recommended for agents. A future
   `strict` profile pairs `prompt` with no default network allowlist and the
   `$HOME`-dotfile allowance fully removed, granting access incrementally
   for high-isolation work.

   The session shell is swapped when the sandbox cannot mediate it: on
   macOS a SIP-protected login shell such as `/bin/zsh` strips the
   mediation library, which would leave the shell's own redirections and
   builtins unmediated. A sandboxed activation runs the interactive bash
   bundled with Flox instead, and sets `SHELL` to the session shell so
   programs that spawn `$SHELL` get a mediated shell too. Set `FLOX_SHELL`
   to a shell the sandbox can mediate — one installed in the environment,
   for example — to use a different shell.

   Requires the `sandbox_activate` feature flag
   (set `FLOX_FEATURES_SANDBOX_ACTIVATE=true`).
   Cannot be combined with in-place activation.
   Mediation is advisory: it only covers cooperative, dynamically linked
   programs, and on macOS does not apply to Apple system binaries that strip
   the injected library. UDP, raw sockets, and statically linked clients are
   not mediated. New-file writes are only checked against their parent
   directory; a write via `rename()` into an out-of-policy location is not
   mediated.
   This is an experimental prototype and may change or be removed.

   Under `prompt`, review and approve queued requests — and inspect or edit
   saved grants — with [`flox-sandbox(1)`](./flox-sandbox.md).

`-g <generation>`, `--generation <generation>`
:  Activate a FloxHub environment at a specific generation.

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
    If `hide_default_prompt` is set to `true`, environments named `default` are
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

# EXAMPLES

Activate an environment stored in the current directory:

```bash
flox activate
```

Activate an environment `some_user/myenv` that's been pushed to FloxHub:

```bash
flox activate -r some_user/myenv
```

Invoke a command inside an environment without entering its subshell:

```bash
flox activate -- cmd --some-arg arg1 arg2
```

Activate `default` Flox environment only within the current shell
(add to the relevant "rc" file, e.g. `~/.bashrc` or `~/.zprofile`):

```bash
eval "$(flox activate)"
```

# SEE ALSO
[`flox-deactivate(1)`](./flox-deactivate.md),
[`flox-activate-allow(1)`](./flox-activate-allow.md),
[`flox-activate-deny(1)`](./flox-activate-deny.md),
[`flox-config(1)`](./flox-config.md),
[`flox-push(1)`](./flox-push.md),
[`flox-pull(1)`](./flox-pull.md),
[`flox-edit(1)`](./flox-edit.md),
[`flox-delete(1)`](./flox-delete.md),
[`flox-sandbox(1)`](./flox-sandbox.md)
