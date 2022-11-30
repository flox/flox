---
title: FLOX-GIT
section: 1
header: "flox User Manuals"
...


# NAME

flox-git - access to the git CLI for floxmeta repository

# SYNOPSIS

flox [ `<general-options>` ] git `<git-subcommand>` [ `<args>` ... ]
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
:   Git subcommand to be invoked

[ `<args>` ... ]
:   Git subcommand arguments

# SEE ALSO

-   *git(1)*
