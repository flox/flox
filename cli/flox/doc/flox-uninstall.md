---
title: FLOX-UNINSTALL
section: 1
header: "Flox User Manuals"
...


# NAME

flox-uninstall - remove packages from an environment

# SYNOPSIS

```
flox [<general options>] (uninstall|rm)
     [-d=<path> | -r=<owner/name>]
     <packages>

```

# DESCRIPTION

Uninstall packages from an environment.

Just like package installation, package uninstallation is transactional.
See [`flox-install(1)`](./flox-install.md) for more details on transactions.
Requesting to uninstall multiple packages where at least one of them was not
previously installed will cause the transaction to fail
and no packages will be uninstalled.

# OPTIONS

## Remove Options

`<packages>`
:   The install IDs of the packages to remove.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`flox-install(1)`](./flox-install.md)
