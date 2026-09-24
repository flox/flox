---
title: FLOX-INSTALL
section: 1
header: "Flox User Manuals"
...


# NAME

flox-install - install packages to an environment

# SYNOPSIS

```text
flox [<general options>] install
     [--pkg-group <name>]
     [--stability <stability>]
     [-i <id>] <package>[@<version>]
     [-i <id>] <package>[^<outputs>]
     [[-i <id>] <package>] ...
```

# DESCRIPTION

Install packages to an environment.

Package installation is transactional.
During an installation attempt the environment is built in order to validate
that the environment isn't broken
(for example, in rare cases packages may provide files that conflict).
If building the environment fails,
including any of the constituent packages,
the attempt is discarded and the environment is unmodified.
If the build succeeds, the environment is atomically updated.

If a requested package is already installed, nothing is done.
If multiple packages are requested and some of them are already installed,
only the new packages are installed and the transaction will still succeed as
long as the build succeeds.

You may also specify packages to be installed via
[`flox-edit(1)`](./flox-edit.md),
which allows specifying a variety of options for package installation.
See [`manifest.toml(5)`](./manifest.toml.md) for more details on the available
options.

## Install ID

The name of a package as it exists in the manifest is referred to as the
"install ID".
This ID is separate from the pkg-path and provides a shorthand for packages
with long names such as `python310Packages.pip`.
Install IDs also provide a way to give packages more semantically meaningful,
convenient, or aesthetically pleasing names in the manifest
(e.g. `node21` instead of `nodejs_21`).
When not explicitly provided, the install ID is inferred based on the pkg-path.
For pkg-paths that consist of a single attribute (e.g. `ripgrep`) the install
ID is set to that attribute.
For pkg-paths that consist of multiple attributes (e.g. `python310Packages.pip`)
the install ID is set to the last attribute in the pkg-path (e.g. `pip`).

As an advanced feature, a Nix flake installable may be specified instead of a
pkg-path,
and in this case the install ID is inferred from the attribute path specified,
or if no attribute path is provided, the install ID is inferred from the flake
reference.

```{.include}
./include/package-names.md
```

# OPTIONS

## Install Options

`-i`, `--id`
:   The install ID of the package as it will appear in the manifest

`--pkg-group <name>`
:   Install the packages into the pkg-group `<name>`.
    Without this option, packages from the Flox Catalog join the `toplevel`
    pkg-group, and packages from custom catalogs get a pkg-group of their own,
    named after their install ID.
    See [`manifest.toml(5)`](./manifest.toml.md) for more on pkg-groups.

`--stability <stability>`
:   Resolve the pkg-group that the packages join against a catalog stability,
    such as `stable` or `lts`,
    by setting `stability` in the manifest's `[pkg-groups]` section.
    The installation fails before the manifest changes
    if the Flox Catalog doesn't provide the stability,
    and the error lists the stabilities that it does provide.

    All packages in a pkg-group share one stability,
    so `--stability` can only set the stability of a pkg-group
    that has no packages yet, or confirm the stability it already has.
    Without `--pkg-group`, packages from the Flox Catalog join the `toplevel`
    pkg-group, so in a new environment
    `flox install --stability <stability> <package>`
    sets the stability of `toplevel`.

    If the pkg-group already has packages
    and its stability is different or unset,
    the installation fails rather than changing the versions of those packages.
    The error shows how to install into a separate pkg-group with
    `--pkg-group`,
    and the `[pkg-groups]` table to set with [`flox-edit(1)`](./flox-edit.md)
    to change the stability of the whole pkg-group instead,
    including the `schema-version` that the table requires
    if the manifest has an older one.
    For a manifest with `version = 1`, the error also lists the packages
    that need `outputs = "all"` to keep installing all of their outputs
    once the manifest has a `schema-version`.
    Packages from included environments count as packages of the pkg-group,
    and a stability set by an included environment counts as its stability.

    A package installed without `--stability` into a pkg-group that has a
    stability resolves against that stability,
    and `flox install` prints which stability that is.

    `--pkg-group` and `--stability` don't move a package that is already
    installed, or change the stability of its pkg-group,
    and `flox install` says so.
    To move it to another pkg-group,
    uninstall it with [`flox-uninstall(1)`](./flox-uninstall.md)
    and install it again with `--pkg-group`.
    To change the stability of the pkg-group that it is in,
    and so of every package in that pkg-group,
    set `stability` in the pkg-group's `[pkg-groups]` table with
    [`flox-edit(1)`](./flox-edit.md).

    `--pkg-group` and `--stability` only apply to packages from a catalog,
    not to flake installables or store paths.

`<package>`
:   The pkg-path of the package to install as shown by 'flox search'.
    Append `@<version>` to specify a version requirement,
    or `^<outputs>` to select which outputs to install.
    Use `^..` to install all outputs,
    or a comma-separated list such as `^bin,man` to install specific outputs.
    The version constraint (`@`) and output specifier (`^`)
    are mutually exclusive.

    Alternatively, an arbitrary Nix flake installable,
    or store path may be specified.
    See [`manifest.toml(5)`](./manifest.toml.md) for more details.


```{.include}
./include/environment-options.md
./include/general-options.md
```

## SEE ALSO
[`flox-uninstall(1)`](./flox-uninstall.md),
[`flox-edit(1)`](./flox-edit.md),
[`manifest.toml(5)`](./manifest.toml.md)
