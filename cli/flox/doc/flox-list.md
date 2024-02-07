---
title: FLOX-LIST
section: 1
header: "Flox User Manuals"
...


# NAME

flox-list - list packages installed in an environment

# SYNOPSIS

```
flox [<general-options>] list
     [-d=<path> | -r=<owner/name>]
     [-c | -n | -e | -a]
```

# DESCRIPTION

List packages installed in an environment.
The options `-n`, `-e`, and `-a` exist to provide varying levels of detail in
the output.

# OPTIONS

## List Options

`-c`, `--config`
:   Show the raw contents of the manifest.

`-n`, `--name`
:   Show only the install ID of each package.

`-e`, `--extended`
:   Show the install ID, pkg-path, and version of each package (default).

`-a`, `--all`
:   Show all available package information including priority and license.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`flox-install(1)`](./flox-install.md)
