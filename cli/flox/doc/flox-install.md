---
title: FLOX-INSTALL
section: 1
header: "Flox User Manuals"
...


# NAME

flox-install - install packages to an environment

# SYNOPSIS

```
flox [<general options>] install
     [-i <id>] <package>
     [[-i <id>] <package>] ...
```

# DESCRIPTION

Install packages to an environment.

Package installation is transactional.
During an installation attempt the environment is built in order to validate
that the environment isn't broken
(for example, in rare cases packages may provide files that conflict).
If building the environment fails,
including any of the consitituent packages,
the attempt is discarded and the environment is unmodified.
If the build succeeds, the environment is atomically updated.

If a requested package is already installed, nothing is done.
If multiple packages are requested and some of them are already installed,
only the new packages are installed and the transaction will still succeed as
long as the build succeeds.

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

You may also specify packages to be installed via
[`flox-edit(1)`](./flox-edit.md),
which allows specifying a variety of options for package installation.
See [`manfifest-toml(1)`](./manifest.toml.md) for more details on the available
options.

```{.include}
./include/package-names.md
```

# OPTIONS

## Install Options

`-i`, `--id`
:   The install ID of the package as it will appear in the manifest.

`<package>`
:   The pkg-path of the package to install as shown by 'flox search'

```{.include}
./include/environment-options.md
./include/general-options.md
```

## SEE ALSO
[`flox-uninstall(1)`](./flox-uninstall.md),
[`flox-edit(1)`](./flox-edit.md),
[`manifest-toml(1)`](./manifest.toml.md)
