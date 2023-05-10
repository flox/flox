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

# OPTIONS

## General options

Many flox commands wrap Nix commands of the same name,
and will correspondingly pass on options and arguments
directly to the underlying `nix` invocation.
For more information on the options supported by specific Nix commands
please invoke `flox nix <command> help`.

The following options are used specifically by `flox`
and must be specified _before_ the `<command>` argument.

-v, \--verbose
:   Verbose mode. Invoke multiple times for increasing detail.

-d, \--debug
:   Debug mode. Invoke multiple times for increasing detail.

-V, \--version
:   Print `flox` version.

\--prefix
:   Print `flox` installation prefix / Nix store path.
    (Flox internal use only.)

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

**search** `<name>` [ (-c|\--channel) `<channel>` ] [ \--verbose ] [ \--refresh ]
:   Search for available packages matching name.

    All channels are searched by default, but if provided
    the `(-c|--channel)` argument can be called multiple times
    to specify the channel(s) to be searched.

    The cache of available packages is updated hourly, but if required
    you can invoke with `--refresh` to update the list before searching.

## Runtime environments

The following options are supported by the commands below.

`(-e|--environment) <name>`
:   Selects **flox environment** to be modified or used. If not provided then
    `flox` will fall back to using the `default` environment.

`--system <type>`
:   Perform command for provided (Nix) system type. If not provided then
    `flox` will fall back to using the current machine's system type.

*Commands*

**create**
:   Create environment. Environments are also automatically created with any of
    `flox (install|import|edit)`.

**install** `<package>` [ `<package>` ... ]
:   Install package(s) to environment.
    See *PACKAGE ARGUMENTS* below for a description of flox package arguments.

**upgrade** `<package>` [ `<package>` ... ]
:   Upgrade package(s) in environment.
    See *PACKAGE ARGUMENTS* below for a description of flox package arguments.

**remove** [ \--force ] `<package>` [ `<package>` ... ]
:   Remove package(s) from environment.
    See *PACKAGE ARGUMENTS* below for a description of flox package arguments.

**import**
:   Import declarative environment manifest as new generation.

**export**
:   Display declarative environment manifest.

**edit**
:   Edit declarative environment manifest. Has the effect of creating the
    environment if it does not exist.

**environments**
:   List all environments.

**generations**
:   List generations of selected environment.

**list** [ \--out-path ] [ \--json ] [ `<generation>` ]
:   List contents of selected environment. Provide optional generation
    argument to list the contents of a specific generation.

**history** [ \--oneline ] [ \--json ]
:   List history of selected environment. With `--oneline` arg, display concise
    format including only the subject line for history log entries.

**activate**
:   Sets environment variables and aliases, runs hooks and adds environment
    `bin` directories to your `$PATH`. Can be invoked from an interactive
    terminal to launch a sub-shell, non-interactively to produce
    a series of commands to be sourced by your current `$SHELL`,
    or with a command and arguments to be invoked directly.

    By default checks for new generations in the background
    and prompts to upgrade with the next activation.
    See `FLOX_AUTOUPDATE` in *ENVIRONMENT VARIABLES* section below
    for further information on configuring the self-updating features
    of flox environments.

    Examples:

    - activate "default" flox environment only within the current shell
    (add to the relevant "rc" file, e.g. `~/.bashrc` or `~/.zprofile`)
    ```
    . <(flox activate)
    ```

    - activate "foo" and "default" flox environments in a new subshell
    ```
    flox activate -e foo
    ```

    - invoke command using "foo" and "default" flox environments
    ```
    flox activate -e foo -- cmd --cmdflag cmdflagarg cmdarg
    ```

**push** / **pull** [ \--force ] [ \--no-render ] [ (-m|--main) ]
:   (`git`) Push or pull metadata to the environment's `floxmeta` repository,
    and in the `pull` case also proceed to render the environment.
    With this mechanism environments can be pushed and pulled between machines
    and within teams just as you would any project managed with `git`.

    With the `--force` argument flox will forceably overwrite either the
    upstream or local copy of the environment based on having invoked
    `push` or `pull`, respectively.

    With the `(-m|--main)` argument `flox (push|pull)` will operate on the
    "floxmain" branch, pushing or pulling user metadata to/from the upstream
    repository. Cannot be used in conjunction with the `-e|--environment` flag.

    With the `--no-render` argument `flox pull` will fetch and incorporate
    the latest metadata from upstream but will not actually render or create
    links to environments in the store. (Flox internal use only.)

**destroy** [ \--origin ] [ \--force ]
:   Remove all local data pertaining to an environment.
    Does *not* remove “upstream” environment data by default.

    Invoke with the `--origin` flag to delete environment data
    both upstream and downstream.

    Invoke with the `--force` flag to avoid the interactive
    confirmation dialog. (Required for non-interactive use.)

## Development

The following option is supported by the commands below.

`(-A|--attr) <package>`
:   Selects package (aka "attrPath") to be used. If not provided `flox`
    will prompt for you to select from the list of known packages.

*Commands*

**build**
:   Build the requested package (or "installable"). If not provided `flox`
    will prompt for you to select from the list of known packages.

**develop**
:   Activate a project flox environment (if it exists) and enter a bash subshell
    with the build environment for the corresponding package.

**print-dev-env**
:   Print a shell script that can be sourced by bash to both activate a project
    flox environment (if it exists) and the build environment for the
    corresponding package. This allows you to enter a development environment in
    your current shell rather than in a subshell (as with flox develop).

**publish**
:   Perform a build, (optionally) copy to cache substituter,
    and render package metadata for inclusion in the flox catalog.

    `[ --build-repo <URL> ]`
    :   The URL of the git repository from which to `flox build` the package.
        This is used both to build the package as it is being published
        and embedded in catalog metadata so that the package can be built
        from source if it cannot be fetched from a binary store.

        (Nix experts will recognize this repository as the source flake
        for the package.)

    `[ --channel-repo <URL> ]`
    :   The URL of the git channel repository to which package
        metadata should be published.
        See **subscribe** and **search** for descriptions on
        the use of channel repositories.

    `[ --upload-to <URL> ]`
    :   The URL of a binary cache location to which built package(s)
        should be copied.

    `[ --download-from <URL> ]`
    :   The URL from which built packages will be served at
        installation time.
        This URL typically refers to the same underlying resource
        as specified by the `--upload-to` argument, but using
        a different transport. For example, we upload packages
        to the (writable, authenticated) s3://flox-store-public URL,
        but users download these packages from the (read-only,
        unauthenticated) https://cache.floxdev.com endpoint.

        If not provided the `--download-from` argument will default to
        the same value as provided for the `--upload-to` argument.

    `[ --render-path <dir> ]`
    :   Sets the directory name for rendering the catalog
        within the git repository
        specified by the `--catalog-repo` flag.
        Defaults to "catalog" if not specified.

    `[ --key-file <file> ]`
    :   Used for identifying the path to the private key
        to be used in signing packages
        before analysis and upload.

    When invoked without arguments, will prompt the user for the required values.

    Once published to a channel repository, you can then
    search for and use your package with the following:

    * subscribe to the channel: `flox subscribe <channel> <URL>`
    * search for a package: `flox search -c <channel> <package>`
    * install a package: `flox install <channel>.<package>`

**run**
:   Run flake application from the requested package (or "installable").
    If not provided `flox` will prompt for you to select from the list of known packages.

## Administration

**config** [ (--list|-l) (--confirm|-c) (--reset|-r) ]
:   Configure and/or display user-specific parameters.

    With the `(--list|-l)` flag will list the current values of all
    configurable parameters.

    With the `(--confirm|-c)` flag will prompt the user to confirm or update
    configurable parameters.

    With the `(--reset|-r)` flag will reset all configurable parameters
    to their default values without further confirmation.

**git** `<git-subcommand>` [ `<args>` ]
:   Direct access to git command invoked in the `floxmeta` repository clone.
    Accepts the `(-e|--environment)` flag for repository selection.
    For expert use only.

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

`$FLOX_AUTOUPDATE`
:   By default flox will check for new environment generations
    in the background as they are activated
    and then prompt to upgrade with the next activation.
    This can be configured as follows:

    Setting `FLOX_AUTOUPDATE=0` disables feature.
    \
    Setting `FLOX_AUTOUPDATE=1` (or if not defined) enables feature.
    \
    Setting `FLOX_AUTOUPDATE=2` configures flox to automatically
    pull updates without prompting.

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
