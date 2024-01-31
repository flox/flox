---
title: FLOX-UPGRADE
section: 1
header: "flox User Manuals"
...


# NAME

flox-upgrade - upgrade packages in an environment

# SYNOPSIS

```
flox [ <general-options> ] upgrade
     [-d=<path> | -r=<owner/name>]
     [<package or group>]...
```

# DESCRIPTION

Upgrade packages in an environment to versions present in the environment's base
catalog.

An upgrade should usually be run after updating an environment's base catalog with
[`flox-update(1)`](./flox-update.md).

When no arguments are specified, all packages in the environment are upgraded.

Packages to upgrade can be specified by either group name,
or, if a package is not in a group with any other packages, it may be specified
by ID.
If the specified argument is both a group name and a package ID, only the group
is upgraded.

Packages without a specified group in the manifest can be upgraded by passing
'toplevel' as the group name.

See [`manifest.toml(1)`](./manifest.toml.md) for more on using package groups.

# OPTIONS

## Upgrade Options

`<package or group>`
:   Package ID or group name of package to upgrade.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO

[`flox-update(1)`](./flox-update.md)
[`manifest.toml(1)`](./manifest.toml.md),
