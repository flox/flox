---
title: FLOX-SHOW
section: 1
header: "Flox User Manuals"
...


# NAME

flox-show - show detailed information about a single package

# SYNOPSIS

flox [ `<general-options>` ] show `<name>` [ \--all ]

# DESCRIPTION

Show detailed information about a single package.

The provided name must be an exact match for a package name i.e. it
must be something you would have copied from the output of the
`flox search` command.

The default output includes the package description, name, latest version,
and license. The `--all` flag will show all versions of the package
that were found in the inputs listed in the manifest.

# OPTIONS

```{.include}
./include/general-options.md
```

## Search Options

[ `<name>` ]
:   Package name to search for

[ \--all ]
:   List all package versions
