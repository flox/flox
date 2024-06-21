---
title: MANIFEST.TOML
section: 5
header: "Flox User Manuals"
...


# NAME

manifest.toml - declarative environment configuration format

# SYNOPSIS

The `manifest.toml` file is a declarative format for specifying the packages
installed in an environment,
environment variables to make available to the environment,
a shell script to run upon activation of the environment,
and other options to change the behavior of the environment.

# DESCRIPTION

Flox environments come with a declarative manifest in
[TOML format](https://toml.io/en/v1.0.0).
An environment can be defined entirely by this one file.
The file is divided into just a few sections that are represented as TOML
tables:

- [`[install]`](#install)
- [`[vars]`](#vars)
- [`[hook]`](#hook)
- [`[profile]`](#profile)
- [`[options]`](#options)

## `[install]`

The `[install]` table is the core of the environment,
specifying which packages you'd like installed in the environment.
An example of the `[install]` table is shown below:

```toml
[install]
ripgrep.pkg-path = "ripgrep"
pip.pkg-path = "python310Packages.pip"
```

Since this is TOML, equivalent ways of writing this would be
```toml
[install]
ripgrep = { pkg-path = "ripgrep" }
pip = { pkg-path = "python310Packages.pip" }
```
or
```
[install.ripgrep]
pkg-path = "ripgrep"

[install.pip]
pkg-path = "python310Packages.pip"
```
Flox will use the first format by default when automatically editing
the manifest.

### Package names

<!-- Copied from package-names.md because it has the wrong header depth -->

Packages are organized in a hierarchical structure such that certain packages
are found at the top level (e.g. `ripgrep`),
and other packages are found under package sets (e.g. `python310Packages.pip`).
We call this location within the catalog the "pkg-path".

The pkg-path is searched when you execute a `flox search` command.
The pkg-path is what's shown by `flox show`.
Finally, the pkg-path appears in your manifest after a `flox install`.

```toml
[install]
ripgrep.pkg-path = "ripgrep"
pip.pkg-path = "python310Packages.pip"
```

### Package descriptors

Each entry in the `[install]` table is a key-value pair.
The key in the key-value pair (e.g. `ripgrep`, `pip`) is referred to as an
"install ID",
and represents the name by which you will refer to a particular package e.g.
if you wanted to uninstall or upgrade the package.
Install IDs are inferred from the last attribute in the pkg-path,
but may also be specified either at install-time via the `-i` option
or interactively via [`flox-edit(1)`](./flox-edit.md).

The value in the key-value pair is called a "package descriptor".
A package is specified by a number of available options which are separate
from the install ID,
so you are free to change them independently of one another.
This allows you to change package details while keeping a stable install ID,
for example upgrading from `gcc.pkg-path = "gcc12"` to
`gcc.pkg-path = "gcc13"`.

The descriptor options allow you to specify in detail the package to install.
The full list of descriptor options are shown below:
```
Descriptor ::= {
  pkg-group          = null | <STRING>
, version            = null | <STRING>
, systems            = null | [<STRING>, ...]
, pkg-path           = <STRING>
, priority           = null | <INT>
}
```

Only `pkg-path` is required.

By specifying some of these options you create a set of requirements that the
installed program must satisfy,
otherwise installation will fail.

By default, all packages belong to the same `pkg-group`, which means providing
specific versions for two different packages can quickly lead to installation
failures.
To avoid such failures, either give a looser `version` constraint,
or move one of the packages to a different package group.

Each option is described below:

`pkg-group`
:   Marks a package as belonging to a pkg-group.
    
    The pkg-group is a collection of software that is known to work together at 
    a point in time. 
    Adding packages to a pkg-group enables packages in the pkg-group to share
    the same libraries and dependencies, which ensures maximum compatibility 
    and minimizes the size of the environment.

    Packages are marked as belonging to a pkg-group simply by setting this
    option to the name of the pkg-group.
    Packages that do not have a pkg-group specified belong to the same group.

    Multiple pkg-groups may resolve to the same version of the catalog.
    Pkg-groups are upgraded as a unit,
    ensuring that the packages within the pkg-group continue to work together.
    See [`flox-upgrade(1)`](./flox-upgrade.md) for more details on how
    pkg-groups and packages interact during upgrades.

`version`
:   Requires that the package match either an exact version or a semver range.

    The semantic version can be specified with the typical qualifiers such as
    `^`, `>=`, etc.
    Semantic versions that do not specify all three fields
    (`MAJOR.MINOR.PATCH`) will treat the unspecified fields as wildcards.
    This instructs Flox to find the latest versions for those fields.
    For example `version = "1.2"` would select the latest version in the
    `1.2.X` series.

`systems`
:   A list of systems on which to install this package.
    When omitted this defaults to the same systems that the manifest
    specifies that it supports via `options.systems`.

`pkg-path`
:   The abbreviated location of a package within a catalog.
    A pkg-path is a sequence of one or more attributes joined by a delimiter.
    For example, both `ripgrep` and `python310Packages.pip` are pkg-paths.
    A pkg-path that contains more than one attribute can be represented as
    either a single string that contains a '.'-delimited sequence of the
    attributes,
    or it can be represented as a TOML array of strings where each string is
    an attribute.
    For example, both `"python310Packages.pip"`
    and `["python310Packages", "pip"]` are equivalent for the `pkg-path`
    option.

    This option is mutually exclusive with `abs-path`.

`priority`
:   A priority used to resolve file conflicts where lower values indicate
    higher priority.

    Each package internally has `/bin`, `/man`, `/include`,
    and other directories for the files they provide.
    These directories from all packages in the
    environment are merged when building the environment.
    Two packages that provide the same `/bin/foo` file cause a conflict,
    and it's ambiguous which file should ultimately be placed into the
    environment.
    Such conflicts can be resolved by assigning different priorities
    to the conflicting packages.

    The default priority is 5.
    Packages with a lower `priority` value will take precedence over packages
    with higher `priority` values.

## `[vars]`

The `[vars]` section allows you to define environment variables for your
environment that are set during environment activation.
The environment variables specified here cannot reference one another.
The names and values of the environment variables are copied verbatim into the
activation script,
so capitalization will be preserved.

Example:
```toml
[vars]
DB_URL = "http://localhost:2000"
SERVER_PORT = "3000"
```

## `[hook]`

The `on-activate` script in the `[hook]` section is useful for performing
initialization in a predictable Bash shell environment.

### `on-activate`

The `on-activate` script is sourced from a **bash** shell,
and it can be useful for spawning processes, dynamically setting environment
variables, and creating files and directories to be used by the subsequent
profile scripts, commands, and shells.

Hook scripts inherit environment variables set in the `[vars]` section,
and variables set here will in turn be inherited by the `[profile]` scripts
described below.

Any output written to `stdout` in a hook script is redirected to `stderr` to
avoid it being mixed with the output of profile section scripts that write to
`stdout` for "in-place" activations.

```toml
[hook]
on-activate = """
    # Interact with the tty as you would in any script
    echo "Starting up $FLOX_ENV_DESCRIPTION environment ..."
    read -e -p "Favourite colour or favorite color? " value

    # Set variables, create files and directories
    venv_dir="$(mktemp -d)"
    export venv_dir

    # Perform initialization steps, e.g. create a python venv
    python -m venv "$venv_dir"

    # Invoke apps that configure the environment via stdout
    eval "$(ssh-agent)"
"""
```

The `on-activate` script is not re-run when activations are nested.
A nested activation can occur when an environment is already active and either
`eval "$(flox activate)"` or `flox activate -- CMD` is run.
In this scenario, `on-activate` is not re-run.
Currently, environment variables set by the first run of the `on-activate`
script are captured and then later set by the nested activation,
but this behavior may change.

It is best to write hooks defensively, assuming the user is using the
environment from any directory on their machine.

### `script` - DEPRECATED
This field was deprecated in favor of the `profile` section.

## `[profile]`

Scripts defined in the `[profile]` section are sourced by *your shell* and
inherit environment variables set in the `[vars]` section and by the `[hook]`
scripts.
The `profile.common` script is sourced for every shell,
and special care should be taken to ensure compatibility with all shells,
after which exactly one of `profile.{bash,fish,tcsh,zsh}` is sourced by the
corresponding shell.

These scripts are useful for performing shell-specific customizations such as
setting aliases or configuring the prompt.

```toml
[profile]
common = """
    echo "it's gettin' flox in here"
"""
bash = """
    source $venv_dir/bin/activate
    alias foo="echo bar"
    set -o vi
"""
zsh = """
    source $venv_dir/bin/activate
    alias foo="echo bar"
    bindkey -v
"""
fish = """
    source $venv_dir/bin/activate.fish
    alias foo="echo bar"
    fish_vi_key_bindings
"""
```

Profile scripts are re-run for nested activations.
A nested activation can occur when an environment is already active and either
`eval "$(flox activate)"` or `flox activate -- CMD` is run.
In this scenario, profile scripts are run a second time.
Re-running profile scripts allows aliases to be set in subshells that inherit
from a parent shell with an already active environment.

## `[options]`

The `[options]` section of the manifest details settings for the environment
itself.

The full set of options are listed below:
```
Options ::= {
  systems                   = null | [<STRING>, ...]
, allow                     = null | Allows
, semver                    = null | Semver
}

Allows ::= {
  unfree   = null | <BOOL>
, broken   = null | <BOOL>
, licenses = null | [<STRING>, ...]
}

Semver ::= {
  allow-pre-releases = <BOOL>
}
```

`systems`
:   The whitelist of systems that this environment supports.
    Valid values are `x86_64-linux`, `aarch64-linux`,
    `x86_64-darwin`, and `aarch64-darwin`.
    [`flox init`](./flox-init.md) automatically populates this list with the
    current system type.
    A user that attempts to pull an environment from FloxHub when their environment
    isn't explicitly supported will be prompted whether to automatically add their
    system to this list.
    See [`flox-pull(1)`](./flox-pull.md) for more details.


`allow.unfree`
:   Allows packages with unfree licenses to be installed and appear in search
    results.
    The default is `false`.

`allow.broken`
:   Allows packages that are marked `broken` in the catalog to be installed and
    appear in search results.
    The default is `false`.

`allow.licenses`
:   A whitelist of software licenses to allow in search results in installs.
    Valid entries are [SPDX Identifiers](https://spdx.org/licenses).

`semver.allow-pre-releases`
:   Whether to allow pre-release software for package installations.
    The default is `false`.
    Setting this value to `true` would allow a package version `4.2.0-pre`
    rather than `4.1.9`.

# SEE ALSO
[`flox-init(1)`](./flox-init.md),
[`flox-install(1)`](./flox-install.md),
[`flox-edit(1)`](./flox-edit.md)
