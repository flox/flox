---
title: FLOX-SEARCH
section: 1
header: "flox User Manuals"
...


# NAME

flox-search - search for packages to install.

# SYNOPSIS

flox [ `<general-options>` ] search `<name>` [ \--refresh ]

# DESCRIPTION

Search for available packages matching name.

The cache of available packages is updated hourly, but if required
you can invoke with `--refresh` to update the list before searching.

# OPTIONS

```{.include}
./include/general-options.md
```

## Search Options

[ `<name>` ]
:   package name to search for

[ \--refresh ]
:   Update the list before searching.

[ \--json ]
:   output the search results in json format

[ -v | \--verbose ]
:   output extended information
