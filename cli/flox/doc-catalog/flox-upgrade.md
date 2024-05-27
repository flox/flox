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
     [<package or pkg-group>]...
```

# DESCRIPTION

Upgrade packages in an environment to versions present in the catalog.

When no arguments are specified, all packages in the environment are upgraded.

Packages to upgrade can be specified by either pkg-group name, or by ID.
If the specified argument is both a pkg-group name and an install ID,
both the package with the install ID
and packages belonging to the pkg-group are upgraded.

Packages without a specified pkg-group in the manifest are placed in a
pkg-group named 'toplevel'.
The packages in that pkg-group can be upgraded without updating any other
pkg-groups by passing 'toplevel' as the pkg-group name.

See [`manifest.toml(5)`](./manifest.toml.md) for more on using pkg-groups.

# OPTIONS

## Upgrade Options

`<package or pkg-group>`
:   Install ID or pkg-group to upgrade.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO

[`flox-update(1)`](./flox-update.md)
[`manifest.toml(5)`](./manifest.toml.md),
