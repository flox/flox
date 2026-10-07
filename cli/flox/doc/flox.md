---
title: FLOX
section: 1
header: "Flox User Manuals"
...

# NAME

flox - developer environments you can take with you

# SYNOPSIS

```text
flox [<general options>] <command>
     [<command options>]
     [<args>] ...
```

# DESCRIPTION

Flox is a virtual environment and package manager all in one.

With Flox you create environments that layer and provide dependencies just
where it matters,
making them portable across the full software lifecycle.

## Command Line Completions

Flox ships with command line completions for `bash`, `fish` and `zsh`.
These completions are installed alongside Flox.

# OPTIONS

```{.include}
./include/general-options.md
```

## Flox Options

`--version`
:   Print `flox` version.

# COMMANDS

Flox commands are grouped into categories pertaining to local development,
sharing environments, and administration.

## Local Development Commands

`init`
:   Create an environment in the current directory.

`activate`
:   Enter the environment, run `flox deactivate` to leave.

`develop`
:   Enter a development shell for a Nix expression build.

`deactivate`
:   Deactivate the current environment.

`run`
:   Run a command from a Flox Catalog package without installing it.

`search`
:   Search for system or library packages to install.

`show`
:   Show details about a single package.

`install`, `i`
:   Install packages into an environment.

`uninstall`
:   Uninstall installed packages from an environment.

`edit`
:   Edit the declarative environment configuration file.

`list`, `l`
:   List packages installed in an environment.

`delete`
:   Delete an environment.

## Sharing Commands

`push`
:   Send an environment to FloxHub.

`pull`
:   Pull an environment from FloxHub.

## Additional Commands

`update`
:   Update an environment's base catalog or update the global base catalog.

`upgrade`
:   Upgrade packages in an environment.

`config`
:   View and set configuration options.

`auth`
:   FloxHub authentication commands.

# ENVIRONMENT VARIABLES

`$FLOX_DISABLE_METRICS`
:   Set to `true` to turn off telemetry.
    Telemetry is the usage events, error reports and performance traces that Flox collects.
    Flox sends usage events to Flox.
    Flox sends error reports and performance traces to Sentry (`sentry.io`), a service that Flox uses.

    Usage events carry a random device ID, and your FloxHub account ID when you are signed in or use a FloxHub token.
    Flox links the device ID to every FloxHub account that signs in on this device.
    Through that link, Flox also ties this device's signed-out events to that account.
    Usage events also carry the search terms you type, your environment names, the packages you install, upgrade or uninstall, and every other item that `disable_metrics` in [`flox-config(1)`](./flox-config.md) lists.
    In an activated shell, Flox's shell prompt hook records two usage events at every prompt.

    Error reports and performance traces carry the device ID and file paths.
    Error reports also carry the error message, and a stack trace when Flox crashes.
    Error reports also carry up to 100 recent Flox log messages, including messages that `-q` hides.
    Error reports from an activation also carry the output of the environment's `hook.on-deactivate` script.
    On macOS, error reports also carry your Mac model.
    Performance traces also carry the search terms, package names, environment names and FloxHub URL of the command.
    Flox and Sentry receive your IP address when Flox sends telemetry.

    This variable overrides `disable_metrics` in every config file.
    It accepts only `true` or `false`, in any letter case.
    Any other value, `1` included, makes Flox commands fail with a configuration error.

    Turning telemetry off does not stop the requests that commands send to FloxHub and the Flox Catalog.
    FloxHub records the search terms and package lists in those requests, with your IP address.
    When you are signed in, FloxHub also records your FloxHub account with them.
    See `disable_metrics` in [`flox-config(1)`](./flox-config.md) for what turning telemetry off stops and what it does not stop.
    For every field that telemetry contains, see [Flox data collection](https://flox.dev/docs/concepts/data-collection).

`$FLOX_MAX_PARALLEL_DOWNLOADS`
:   Variable for controlling parallel downloads when building an environment.
    By default, every package for an environment is downloaded in parallel, but
    on poor connections this can cause packages to download slower and in some
    cases time out. Setting this to a number greater than or equal to 1 will
    limit the number of active downloads to the specified number.

`$EDITOR`, `$VISUAL`
:   Override the default editor used for editing environment manifests and commit messages.

`$SSL_CERT_FILE`, `$NIX_SSL_CERT_FILE`
:   If set, overrides the path to the default Flox provided SSL certificate bundle.
    Set `NIX_SSL_CERT_FILE` to only override packages built with Nix,
    and otherwise set `SSL_CERT_FILE` to override the value for all packages.

    See also: [Nix environment variables - `NIX_SSL_CERT_FILE`](https://nixos.org/manual/nix/stable/installation/env-variables.html#nix_ssl_cert_file)

# SEE ALSO

[`flox-init(1)`](./flox-init.md),
[`flox-activate(1)`](./flox-activate.md),
[`flox-develop(1)`](./flox-develop.md),
[`flox-run(1)`](./flox-run.md),
[`flox-install(1)`](./flox-install.md),
[`flox-uninstall(1)`](./flox-uninstall.md),
[`flox-upgrade(1)`](./flox-upgrade.md),
[`flox-search(1)`](./flox-search.md),
[`flox-show(1)`](./flox-show.md),
[`flox-edit(1)`](./flox-edit.md),
[`manifest.toml(5)`](./manifest.toml.md),
[`flox-list(1)`](./flox-list.md),
[`flox-auth(1)`](./flox-auth.md),
[`flox-push(1)`](./flox-push.md),
[`flox-pull(1)`](./flox-pull.md),
[`flox-delete(1)`](./flox-delete.md),
[`flox-config(1)`](./flox-config.md)
