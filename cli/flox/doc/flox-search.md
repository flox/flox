---
title: FLOX-SEARCH
section: 1
header: "Flox User Manuals"
...


# NAME

flox-search - search for packages

# SYNOPSIS

```
flox [<general options>] search
     [--json]
     [-a]
     [--binary]
     <search-term>
```

# DESCRIPTION

Search for available packages.

A limited number of search results are reported by default for brevity.
The full result set can be returned via the `-a` flag.

Only the package name and description are shown by default.
Structured search results can be returned via the `--json` flag.
More specific information for a single package is available via the
[`flox-show(1)`](./flox-show.md) command.

```{.include}
./include/package-names.md
```

## Fuzzy search
`flox search` uses a fuzzy search mechanism that tries to match either some
portion of the pkg-path or description.

# OPTIONS

## Search Options

`<search-term>`
:   The package name to search for.

`--json`
:   Display the search results in JSON format.

`-a`, `--all`
:   Display all search results (default: at most 10).

`--binary`
:   Search for packages that provide a specific binary.
    Instead of matching package names and descriptions,
    this searches for packages whose `bin/` directory contains
    the specified binary name.
    For example, `flox search --binary rg` finds `ripgrep`.

```{.include}
./include/general-options.md
```

# EXAMPLES

Search for a package by name:

```
$ flox search curl
```

Search for packages that provide a binary:

```
$ flox search --binary rg
```

# SEE ALSO
[`flox-show(1)`](./flox-show.md),
[`flox-run(1)`](./flox-run.md)
