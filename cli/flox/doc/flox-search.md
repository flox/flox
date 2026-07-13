---
title: FLOX-SEARCH
section: 1
header: "Flox User Manuals"
...


# NAME

flox-search - search for packages

# SYNOPSIS

```text
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

## Searching by binary
With the `--binary` flag, the search term is treated as a binary name
and matched against the FloxHub binary-to-package index instead of
package names and descriptions.
This finds packages whose outputs contain the named binary,
even when the package is named differently:

```bash
$ flox search --binary rg
ripgrep    Utility that combines the usability of The Silver Searcher with ...
```

This is the same lookup [`flox-run(1)`](./flox-run.md) performs when
invoked without `--package`.

# OPTIONS

## Search Options

`<search-term>`
:   The package name to search for.
    With `--binary`, the binary name to look up.

`--json`
:   Display the search results in JSON format.

`-a`, `--all`
:   Display all search results (default: at most 10).

`--binary`
:   Search for packages that provide a specific binary
    instead of matching package names and descriptions.

```{.include}
./include/general-options.md
```

# SEE ALSO
[`flox-show(1)`](./flox-show.md),
[`flox-run(1)`](./flox-run.md)
