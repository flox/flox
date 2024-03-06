---
title: MANIFEST.TOML
section: 1
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
  name               = null | <STRING>
, optional           = null | <BOOL>
, pkg-group          = null | <STRING>
, version            = null | <STRING>
, semver             = null | <STRING>
, systems            = null | [<STRING>, ...]
, pkg-path           = null | <STRING> | [<STRING>, ...]
, abs-path           = null | <STRING> | [<STRING>, ...]
, priority           = null | <INT>
}
```

None of these options are required,
and leaving them unset instructs Flox to simply find the best match for the
package name and latest version given the install ID.
If you omit all options,
setting `package = {}`,
the install ID will be used as the pkg-path.
This behavior is subject to change and is not recommended.

By specifying some of these options you create a set of requirements that the
installed program must satisfy,
otherwise installation will fail.
The most common option will likely be the `semver` option,
which allows you to specify a semantic version range.

Each option is described below:

`name`
:   Matches either the last attribute of the `pkg-path` or the package metadata
    fields `name` or `pname` as set by the catalog.
    This option is mutually exclusive with the `pkg-path` and `abs-path`
    options.
    You shouldn't need to use this option and should instead prefer the
    `pkg-path` option.

`optional`
:   Marks this package as an optional requirement for the environment.
    By default an environment will fail to build if a specified package can't
    be found in the catalog.
    However, some packages are platform specific and will never be found in the
    catalog for some systems.
    Thus, the way you mark a package as platform specific is by setting
    `optional = true` or using the `systems` option to list the systems on
    which the package is required.

`pkg-group`
:   Marks a package as belonging to a pkg-group.

    Adding packages to a pkg-group ensures all packages in the pkg-group share
    the same libraries and dependencies,
    which ensures maximum compatibility and minimizes the size of the
    environment.
    One example is C/C++ projects that depend on specific versions of header
    files.
    Packages are marked as belonging to a pkg-group simply by setting this
    option to the name of the pkg-group.

    Multiple pkg-groups may resolve to the same version of the catalog.
    Pkg-groups are upgraded as a unit,
    ensuring that the packages within the pkg-group continue to work together.
    See [`flox-upgrade(1)`](./flox-upgrade.md) for more details on how
    pkg-groups and packages interact during upgrades.

`version`
:   Requires that the package match either an exact version or a semver range.
    When the first character of the `version` string is '=' the version must be
    an exact match,
    otherwise the `version` string is matched as a semver range.
    Versions that don't conform to semver must be specified with '='.
    
    The semantic version can be specified with the typical qualifiers such as
    `^`, `>=`, etc.
    Semantic versions that do not specify all three fields
    (`MAJOR.MINOR.PATCH`) will treat the unspecified fields as wildcards.
    This instructs Flox to find the latest versions for those fields.
    For example `version = "1.2"` would select the latest version in the
    `1.2.X` series.

    This option is mutually exclusive with the `semver` option.

`semver`
:   This option is similar to `version` except it _only_ allows semantic
    versions.

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

`abs-path`
:   The fully-qualified location of a package within a catalog.
    For the `ripgrep` package in the Flox base catalog for an `x86_64-linux`
    system this would be `legacyPackages.x86_64-linux.ripgrep`.
    Note that "legacyPackages" has nothing to do packages being out of date,
    and instead has to do with internal Flox implementation details.
    The abs-path can be specified for all systems by using `*` or `null` as
    the system.
    
    You should rarely ever need this option and should instead prefer the
    `pkg-path` option.

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

The `[hook]` section of the manifest allows you to specify a script that's
executed immediately after the environment is activated.
Since the hook runs after activation, the environment variables in the `[vars]`
section may be referenced within the hook.

Common usages for environment hooks are printing usage messages or performing
setup operations such as initializing a database or starting a server.

### `on-activate`
The `on-activate` script is run non-interactively in a Bash subshell after the
environment is activated.
This is useful for environment initialization that you want done in a consistent
shell so that you don't need to worry about shell compatibility.
The exit code and `stdout` of this script are discarded.

```toml
[hook]
on-activate = """
    mkdir -p data_dir
"""
```


### `script`
This `script` option defines a script that is sourced by the user's interactive
shell.
This is the main difference between `hook.script` and `hook.on-activate`.

```toml
[hook]
script = """
    # Start the development server
    start_server --port "$SERVER_PORT" --db-url "$DB_URL"

    echo "Server started on port $SERVER_PORT"
"""
```

## `[options]`

The `[options]` section of the manifest details settings for the environment
itself.

The most common option to set is `systems`,
which specifies which systems the environment supports.
A user that attempts to pull an environment from FloxHub when their environment
isn't explicitly supported will be prompted whether to automatically add their
system to this list.
See [`flox-pull(1)`](./flox-pull.md) for more details.

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
  prefer-pre-releases = <BOOL>
}
```

`systems`
:   The whitelist of systems that this environment supports.
    Valid values are `x86_64-linux`, `aarch64-linux`,
    `x86_64-darwin`, and `aarch64-darwin`.
    [`flox init`](./flox-init.md) automatically populates this list with the
    current system type.

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

`semver.prefer-pre-releases`
:   Whether to prefer pre-release software over stable versions for the
    purposes of search results and package installations.
    The default is `false`.
    Setting this value to `true` would prefer a package version `4.2.0-pre`
    over `4.1.9`.

# SEE ALSO
[`flox-init(1)`](./flox-init.md),
[`flox-install(1)`](./flox-install.md),
[`flox-edit(1)`](./flox-edit.md)
