---
title: FLOX-CONFIG
section: 1
header: "flox User Manuals"
...


# NAME

flox-config - access to the gh CLI

# SYNOPSIS

flox [ `<general-options>` ] config `<gh-subcommand>` [ `<args>` ]
# DESCRIPTION

Direct access to git command invoked in the `floxmeta` repository clone.
Accepts the `(-e|--environment)` flag for repository selection.

**For expert use only.**

# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```

## Git Options

`<git-subcommand>`
:   gh subcommand to be invoked

[ `<args>` ... ]
:   gh subcommand arguments

# SEE ALSO

-   *gh(1)*
