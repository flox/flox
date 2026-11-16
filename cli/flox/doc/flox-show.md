---
title: FLOX-SHOW
section: 1
header: "Flox User Manuals"
...


# NAME

flox-show - show detailed information about a single package

# SYNOPSIS

```text
flox [<general-options>] show [--all] [--json] <pkg-path>
```

# DESCRIPTION

Show detailed information about a single package.

`flox show` queries the Flox Catalog and does not require a FloxHub account.

The default output includes the package description,
available outputs (with defaults marked by `*`),
and available versions.
With `--all`, each version also lists the stabilities that have it,
strictest first.
To install a version, its pkg-group must use one of those stabilities.

```{.include}
./include/package-names.md
```

# OPTIONS

```{.include}
./include/general-options.md
```

## Show Options

`--all`
:   Show additional metadata about each version, including stabilities that provide each version.
    Ignored with `--json`.

`--json`
:   Display available versions as a JSON array with all available metadata about each version.

`<pkg-path>`
:   Package name to show details for.

# EXAMPLES

Display detailed information about the `ripgrep` package:

```console
$ flox show ripgrep
ripgrep - A utility that combines the usability of The Silver Searcher with the raw speed of grep
    ripgrep@13.0.0
    ripgrep@14.1.0
```

# SEE ALSO
[`flox-search(1)`](./flox-search.md),
[`flox-install(1)`](./flox-install.md)
