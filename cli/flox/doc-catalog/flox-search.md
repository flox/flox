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
     <search-term>
```

# DESCRIPTION

Search for available packages.

Searches are performed in the context of the environment if one exists,
making use of the environment's lock file and the locked base catalog within it
if either one exists.
Searches performed outside of an environment query a global base catalog.
Both the global and environment's base catalogs can be updated with
[`flox-update(1)`](./flox-update.md).

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
`flox search` uses a fuzzy search mechanism that tries to match either the
package name itself or some portion of the pkg-path.

# OPTIONS

## Search Options

`<search-term>`
:   The package name to search for.

`--json`
:   Display the search results in JSON format.

`-a`, `--all`
:   Display all search results (default: at most 10).

```{.include}
./include/general-options.md
```

# SEE ALSO
[`flox-show(1)`](./flox-show.md),
[`flox-update(1)`](./flox-update.md)
