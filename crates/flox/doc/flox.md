% FLOX(1) flox User Manuals

# NAME

flox - command-line interface (CLI)

# SYNOPSIS

flox [ `<options>` ] command [ `<command options>` ] [ `<args>` ] ...

# DESCRIPTION

flox is a platform for developing, building, and using packages created with Nix.
It can be used alone to simplify the process of working with Nix,
within a team for the sharing of development environments,
and in enterprises as system development lifecycle (SDLC) framework.

The `flox` CLI is used:

1. To manage and use collections of packages,
   environment variables and services
   known as flox *runtime environments*,
   which can be used in a variety of contexts
   on any Linux distribution, in or out of a container.
1. To launch flox *development environments*
   as maintained using a `flox.toml` file
   stored within a project directory.
1. As a wrapper for Nix functionality
   which drives the process of building packages with flox.

<!--
See *floxtutorial(7)* to get started.
More in-depth information is available by way of the [flox User's Manual](https://alpha.floxsdlc.com/docs/).
-->

## Command Line Completions

Flox ships with command line completions for `bash`, `zsh` and `fish`.
To enable completions in your shell Add the follwoing to your shell's
startup script:

**When using the flox installers**

- `bash`, `zsh`
```
export XDG_DATA_DIRS="/usr/local/share:$XDG_DATA_DIRS"
```

- `fish`
```
set -px XDG_DATA_DIRS "/usr/local/share"
```

**When installing flox through Nix**

- `bash`, `zsh`
```
# for single user user installs
export XDG_DATA_DIRS="$HOME/.nix-profile/share:$XDG_DATA_DIRS"

# for global installs
export XDG_DATA_DIRS="/nix/var/nix/profiles/default:$XDG_DATA_DIRS"

```

- `fish`
```
# for single user user installs
set -px XDG_DATA_DIRS "$HOME/.nix-profile/share"

# for global installs
set -px XDG_DATA_DIRS "/nix/var/nix/profiles/default"
```

# OPTIONS

```{.include}
./include/general-options.md
```

# COMMANDS

Flox commands are grouped into categories pertaining to
runtime environments, developer environments, and administration.

## Packages

**channels**
:   List channel subscriptions.

**subscribe** [ `<name>` [ `<url>` ] ]
:   Subscribe to a flox package channel.
    If provided, will register the name to the provided URL,
    and will otherwise prompt with suggested values.

**unsubscribe** [ `<name>` ]
:   Unsubscribe from the named channel.
    Will prompt for the channel name if not provided.

**search** `<name>` [ (-c|\--channel) `<channel>` ] [ \--refresh ]
:   Search for available packages matching name.

## Runtime environments

**install** `<package>` [ `<package>` ... ]
:   Install package(s) to environment.

**upgrade** `<package>` [ `<package>` ... ]
:   Upgrade package(s) in environment.

**remove** [ \--force ] `<package>` [ `<package>` ... ]
:   Remove package(s) from environment.

**import**
:   Import declarative environment manifest as new generation.

**export**
:   Display declarative environment manifest.

**edit**
:   Edit declarative environment manifest.

**environments**
:   List all environments.

**generations**
:   List generations of selected environment.

**list** [ `<generation>` ]
:   List contents of selected environment.

**history** [ \--oneline ]
:   List history of selected environment.

**activate**
:   Sets environment variables and aliases, runs hooks and adds environment
    `bin` directories to your `$PATH`.

**push** / **pull** [ \--force ]
:   (`git`) Push or pull metadata to the environment's `floxmeta` repository.

**destroy** [ \--origin ] [ \--force ]
:   Remove all local data pertaining to an environment.

## Development

**build**
:   Build the requested package (or "installable").

**develop**
:   Launch subshell configured for development environment using the
    `flox.toml` or Nix expression file as found in the current directory.


**publish**
:   Perform a build, (optionally) copy binaries to a cache,
    and add package metadata to a flox channel.

**run**
:   Run flake application from the requested package (or "installable").

## Administration

**config** [ (--list|-l) (--confirm|-c) (--reset|-r) ]
:   Configure and/or display user-specific parameters.

**git** `<git-subcommand>` [ `<args>` ]
:   Direct access to git command invoked in the `floxmeta` repository clone.

**gh** `<gh-subcommand>` [ `<args>` ]
:   Direct access to gh command. For expert use only.

# PACKAGE ARGUMENTS

Flox package arguments are specified as a tuple of
stability, channel, name, and version in the format:
`<stability>`.`<channel>`.`<name>`@`<version>`

The version field is optional, defaulting to the latest version if not specified.

The stability field is also optional, defaulting to "stable" if not specified.

The channel field is also optional, defaulting to "nixpkgs-flox" if not specified,
_but only if using the "stable" stability_. If using anything other than the
default "stable" stability, the channel *must* be specified.

For example, each of the following will install the latest hello version 2.12 from
the stable channel:
```
flox install stable.nixpkgs-flox.hello@2.12
flox install stable.nixpkgs-flox.hello
flox install nixpkgs-flox.hello@2.12
flox install nixpkgs-flox.hello
flox install hello@2.12
flox install hello
```

... and each of the following will install the older hello version 2.10
from the stable channel:
```
flox install stable.nixpkgs-flox.hello@2.10
flox install nixpkgs-flox.hello@2.10
flox install hello@2.10
```

... but only the following will install the older hello version 2.10 from the unstable channel:
```
flox install unstable.nixpkgs-flox.hello@2.10
```

# ENVIRONMENT VARIABLES

`$FLOX_HOME`
:   Location for runtime flox environments as included in `PATH` environment variable.
    Defaults to `$XDG_DATA_HOME/flox/environments` or `$HOME/.local/share/flox/environments`
    if `$XDG_DATA_HOME` is not defined.

`$FLOX_PROMPT`, `$FLOX_PROMPT_COLOR_{1,2}`, `$FLOX_PROMPT_DISABLE`
:   The **FLOX_PROMPT** variable defaults to a bold blue "flox"
    and can be used to specify an alternate flox indicator string
    (including fancy colors, if desired).
    For example, include the following in your `~/.bashrc` and/or `~/.zshprofile`
    (or equivalent) to display the flox indicator in bold green:

    bash: `export FLOX_PROMPT="\[\033[1;32m\]flox\[\033[0m\] "`
    \
    zsh: `export FLOX_PROMPT='%B%F{green}flox%f%b '`

    If you're just looking to pick different colors,
    the **FLOX_PROMPT_COLOR_1** and **FLOX_PROMPT_COLOR_2** variables
    can be used to select the color of the
    "flox" and activated environments portions of the prompt, respectively.
    The values of these variables should be integers
    chosen from the 256-color palette as described in the
    [xterm-256color chart](https://upload.wikimedia.org/wikipedia/commons/1/15/Xterm_256color_chart.svg).
    For example, setting `FLOX_PROMPT_COLOR_1=32` will result in the same
    prompt as in the examples above.

    If defined, the **FLOX_PROMPT_DISABLE** variable prevents
    flox from performing all prompt customization for interactive shells.

`$FLOX_VERBOSE`
:   Setting **FLOX_VERBOSE=1** is the same as invoking `flox` with the `--verbose`
    argument except that it can be convenient to set this in the environment for
    the purposes of development.

`$FLOX_DEBUG`
:   Setting **FLOX_DEBUG=1** is the same as invoking `flox` with the `--debug`
    argument except that it activates debugging prior to the start of argument
    parsing and that it can be convenient to set this in the environment for
    the purposes of development.

`$FLOX_METRICS`
:   Location for the flox metrics accumulator file.
    Defaults to `$XDG_DATA_HOME/.cache/metrics-events` or `$HOME/.cache/metrics-events`
    if `$XDG_DATA_HOME` is not defined.

`$FLOX_DISABLE_METRICS`
:   Variable for disabling the collection/sending of metrics data.
    If not empty, prevents flox from submitting basic metrics information
    including the subcommand issued along with a unique token.

`$EDITOR`, `$VISUAL`
:   Override the default editor used for editing environment manifests and commit messages.

`$SSL_CERT_FILE`, `$NIX_SSL_CERT_FILE`
:   If set, overrides the path to the default flox-provided SSL certificate bundle.
    Set `NIX_SSL_CERT_FILE` to only override packages built with Nix,
    and otherwise set `SSL_CERT_FILE` to override the value for all packages.

    See also: https://nixos.org/manual/nix/stable/installation/env-variables.html#nix_ssl_cert_file

<!--
# EXAMPLES

# SEE ALSO

`flox-framework`(7)

`flox-tutorial`(7)
-->
