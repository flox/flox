---
title: FLOX-SEARCH
section: 1
header: "flox User Manuals"
...


# NAME

flox-search - search packages in subscribed channels.

# SYNOPSIS

flox [ `<general-options>` ] search `<name>` [ (-c|\--channel) `<channel>` ]... [ \--refresh ]

# DESCRIPTION

Search for available packages matching name.

All channels are searched by default, but if provided
the `(-c|--channel)` argument can be called multiple times
to specify the channel(s) to be searched.

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
