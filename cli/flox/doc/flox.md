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
:   Variable for disabling the collection/sending of metrics data.
    If set to `true`, prevents Flox from submitting basic metrics information
    such as a unique token and the subcommand issued.

`$FLOX_MAX_PARALLEL_DOWNLOADS`
:   Variable for controlling parallel downloads when building an environment.
    By default, every package for an environment is downloaded in parallel, but
    on poor connections this can cause packages to download slower and in some
    cases time out. Setting this to a number greater than or equal to 1 will
    limit the number of active downloads to the specified number.

`$EDITOR`, `$VISUAL`
:   Override the default editor used for editing environment manifests and commit messages.

`$SSL_CERT_FILE`, `$NIX_SSL_CERT_FILE`
:   Certificate bundles for TLS. `SSL_CERT_FILE` is the system-wide setting
    that every program honors, and Flox never sets it. `NIX_SSL_CERT_FILE`
    is read by Nix and by Nix-built software (ahead of `SSL_CERT_FILE`); set
    it to give those a bundle of their own, for example when `SSL_CERT_FILE`
    is in a format they cannot read. If neither is set, Flox sets
    `NIX_SSL_CERT_FILE` to the bundle it ships, for Nix software alone, and
    exports it into activated environments. Once an activation has applied
    that default, an `SSL_CERT_FILE` exported later in the shell does not
    reach Nix-built software; set `NIX_SSL_CERT_FILE` as well. For Nix
    itself, an `ssl-cert-file` setting in `nix.conf` outranks both
    variables. Flox does not pass `SSL_CERT_DIR` on to Nix-built software
    either; an environment that needs a directory of certificates can
    export it from `[vars]`.

    See also: [Nix environment variables - `NIX_SSL_CERT_FILE`](https://nixos.org/manual/nix/stable/installation/env-variables.html#nix_ssl_cert_file)

`$LOCALE_ARCHIVE` (Linux), `$PATH_LOCALE` (macOS)
:   Locale data for Nix-built software, which does not find the system's
    own. If unset, Flox sets the variable to the locale data it ships and
    exports it into activated environments. A value you set is left alone.

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
