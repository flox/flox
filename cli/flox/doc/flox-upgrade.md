---
title: FLOX-UPGRADE
section: 1
header: "Flox User Manuals"
...


# NAME

flox-upgrade - upgrade packages in an environment

# SYNOPSIS

```
flox [<general-options>] upgrade
     [-d=<path> | -r=<owner>/<name>]
     [--dry-run]
     [<package or pkg-group>]...
```

# DESCRIPTION

Upgrade packages in the environment.

When no arguments are specified,
all packages in the environment are upgraded if possible.
A package is upgraded if its version, build configuration,
or dependency graph changes.

Packages to upgrade can be specified by group name.
Packages without a specified pkg-group in the manifest
are placed in a group named 'toplevel'.
The packages in that group can be upgraded without updating any other groups
by passing 'toplevel' as the group name.

A single package can only be specified to upgrade by ID
if it is not in a group with any other packages.

See [`manifest.toml(5)`](./manifest.toml.md) for more on using pkg-groups.

# OPTIONS

## Upgrade Options

`--dry-run`
:   Show available upgrades but do not apply them.

`<package or pkg-group>`
:   Install ID or pkg-group to upgrade.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO

[`manifest.toml(5)`](./manifest.toml.md)
