---
title: FLOX-SHOW
section: 1
header: "Flox User Manuals"
...


# NAME

flox-show - show detailed information about a single package

# SYNOPSIS

```
flox [<general-options>] show <pkg-path>
```

# DESCRIPTION

Show detailed information about a single package.

The default output includes the package description,
name,
and version.

```{.include}
./include/package-names.md
```

# OPTIONS

```{.include}
./include/general-options.md
```

## Show Options

`<pkg-path>`
:   Package name to show details for.

# EXAMPLES:

Display detailed information about the `ripgrep` package:
```
$ flox show ripgrep
ripgrep - A utility that combines the usability of The Silver Searcher with the raw speed of grep
    ripgrep@13.0.0
    ripgrep@14.1.0
```

# SEE ALSO
[`flox-search(1)`](./flox-search.md),
[`flox-install(1)`](./flox-install.md)
