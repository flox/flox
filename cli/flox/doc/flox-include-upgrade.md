---
title: FLOX-INCLUDE-UPGRADE
section: 1
header: "Flox User Manuals"
...

# NAME

flox-include-upgrade - upgrade an environment with latest changes to its
included environments

# SYNOPSIS

```text
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

A path environment already uses the latest changes that the path
environments it includes with `dir` have locked, but keeps them out of its
lockfile.
This command saves them to the lockfile,
and gets the latest contents of other included environments, such as those
included with `remote`.

# OPTIONS

`<included environment>`
:   Name of included environment to check for changes

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`manifest.toml(5)`](./manifest.toml.md)
