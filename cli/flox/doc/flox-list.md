---
title: FLOX-LIST
section: 1
header: "Flox User Manuals"
...


# NAME

flox-list - list packages installed in an environment

# SYNOPSIS

flox [ `<general-options>` ] list [ (\--out-path | \--json) ] [ `<generation>` ]

# DESCRIPTION

List contents of selected environment.
Provide optional generation argument to list the contents
of a specific generation.

# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```

## List Options

[ \--out-path ]
:   Include store paths of packages in the environment


[ \--json ]
:   Print as machine readable JSON

[ `<generation>` ]
:   Generation to list, defaults to the latest if absent.
