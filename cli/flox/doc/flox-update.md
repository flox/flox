---
title: FLOX-UPDATE
section: 1
header: "Flox User Manuals"
...

> **Warning:**
> This command is **deprecated** and no longer supported

# NAME

flox-update - update the global base catalog or an environment's base catalog

# SYNOPSIS

```
flox [<general-options>] update
     [--global | (-d=<path> | -r=<owner>/<name>)]
```

# DESCRIPTION

Update an environment's base catalog,
or update the global base catalog if `--global` is specified.

The base catalog is a collection of packages used by various flox subcommands.

The global base catalog provides packages for
[`flox-search(1)`](./flox-search.md) and [`flox-show(1)`](./flox-show.md) when
not using an environment,
and it is used to initialize an environment's base catalog.

An environment's base catalog provides packages for
[`flox-search(1)`](./flox-search.md) and [`flox-show(1)`](./flox-show.md) when
using that environment,
and it provides packages for [`flox-install(1)`](./flox-install.md) and
[`flox-upgrade(1)`](./flox-upgrade.md).

Note that updating an environment's base catalog and upgrading packages are two
separate options.
Upgrading packages will usually require running an update command followed by a
[`flox-upgrade`](./flox-upgrade.md).

# OPTIONS

## Update Options

`--global`
:   Update the global base catalog

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO

[`flox-upgrade(1)`](./flox-upgrade.md)
