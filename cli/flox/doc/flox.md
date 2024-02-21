---
title: FLOX
section: 1
header: "Flox User Manuals"
...

# NAME

flox - developer environments you can take with you

# SYNOPSIS

```
flox [<general options>] <command>
     [<command options>]
     [<args>] ...
```

# DESCRIPTION

Flox is a virtual environment and package manager all in one.

With flox you create environments that layer and provide dependencies just
where it matters,
making them portable across the full software lifecycle.

## Command Line Completions

Flox ships with command line completions for `bash`, `zsh` and `fish`.
These completions are installed alongside Flox.

# OPTIONS

```{.include}
./include/general-options.md
```

## flox Options

`--version`
:   Print `flox` version.

# COMMANDS

Flox commands are grouped into categories pertaining to local development,
sharing environments, and administration.

## Local Development Commands

`init`
:   Create an environment in the current directory.

`activate`
:   Enter the environment, type `exit` to leave.

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

`list`
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
:   Variable for disabling the collection/sending of metrics data.
    If set to `true`, prevents Flox from submitting basic metrics information
    such as a unique token and the subcommand issued.

`$EDITOR`, `$VISUAL`
:   Override the default editor used for editing environment manifests and commit messages.

`$SSL_CERT_FILE`, `$NIX_SSL_CERT_FILE`
:   If set, overrides the path to the default flox-provided SSL certificate bundle.
    Set `NIX_SSL_CERT_FILE` to only override packages built with Nix,
    and otherwise set `SSL_CERT_FILE` to override the value for all packages.

    See also: [Nix environment variables - `NIX_SSL_CERT_FILE`](https://nixos.org/manual/nix/stable/installation/env-variables.html#nix_ssl_cert_file)

# SEE ALSO

[`flox-init`(1)](./flox-init.md),
[`flox-activate`(1)](./flox-activate.md),
[`flox-install`(1)](./flox-install.md),
[`flox-uninstall(1)`](./flox-uninstall.md),
[`flox-update(1)`](./flox-update.md),
[`flox-upgrade`(1)](./flox-upgrade.md),
[`flox-search`(1)](./flox-search.md),
[`flox-show(1)`](./flox-show.md),
[`flox-edit`(1)](./flox-edit.md),
[`flox-list`(1)](./flox-list.md),
[`flox-auth(1)`](./flox-auth.md),
[`flox-push`(1)](./flox-push.md),
[`flox-pull`(1)](./flox-pull.md),
[`flox-delete`(1)](./flox-delete.md),
[`flox-config`(1)](./flox-config.md)
