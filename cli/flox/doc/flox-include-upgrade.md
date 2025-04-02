---
title: FLOX-INCLUDE-UPGRADE
section: 1
header: "Flox User Manuals"
...

# NAME

flox-include-upgrade - upgrade an environment with latest changes to its
included environments

# SYNOPSIS

```
flox [<general-options>] include upgrade
     [-d=<path> | -r=<owner/name>]
     [<included environment>]...
```

# DESCRIPTION

Get the latest contents of included environments and merge them with the
composing environment.

If the names of specific included environments are provided, only changes for
those environments will be fetched. If no names are provided, changes will be
fetched for all included environments.

# OPTIONS

`<included environment>`
:   Name of included environment to check for changes

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`manifest-toml`(5)](./manifest.toml.md),
