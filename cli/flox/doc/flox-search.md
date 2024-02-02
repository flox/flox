---
title: FLOX-SEARCH
section: 1
header: "flox User Manuals"
...


# NAME

flox-search - search for packages.

# SYNOPSIS

```
flox [ <general options> ] search
     [ --json ]
     [ -a]
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

# OPTIONS

## Search Options

`<search-term>`
:   package name to search for

`--json`
:   output the search results in json format

`-a`, `--all`
:   display all search results (default: limited number)

```{.include}
./include/general-options.md
```

# See also
[`flox-show(1)`](./flox-show.md),
[`flox-update(1)`](./flox-update.md)
