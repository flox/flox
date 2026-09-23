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
    such as `lts`, `stable`, `staging`, or `unstable`,
    by setting `stability` in the manifest's `[pkg-groups]` section.

    All packages in a pkg-group share one stability.
    If the pkg-group already contains packages that use a different stability,
    the installation fails rather than re-resolving them;
    use `--pkg-group` to install into a separate pkg-group,
    or change the pkg-group's stability with [`flox-edit(1)`](./flox-edit.md).

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
