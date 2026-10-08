---
title: FLOX-LIST
section: 1
header: "Flox User Manuals"
...


# NAME

flox-list - list packages installed in an environment

# SYNOPSIS

```text
flox [<general-options>] list
     [-d=<path> | -r=<owner/name>]
     [-u]
     [-e | -c | -n | -a]
```

# DESCRIPTION

List packages installed in an environment.
The options `-n`, `-e`, and `-a` exist to provide varying levels of detail in
the output.

# OPTIONS

## List Options

`-e`, `--extended`
:   Show the install ID, pkg-path, and version of each package (default).

`-c`, `--config`
:   Show the raw contents of the manifest.
    When using composition, the merged manifest will be shown without any
    commented lines.

`-n`, `--name`
:   Show only the install ID of each package.

`-a`, `--all`
:   Show information about the environment as a whole — name,
    generation, FloxHub sync status, upgrade-notification setting —
    followed by full details for each package including priority,
    license, available outputs, and whether an upgrade is pending.
    Next-step hints are printed to stderr.
    This flag makes zero network requests;
    all information is derived from on-disk state.

```{.include}
./include/environment-options.md
./include/upstream-option.md
./include/general-options.md
```

# SEE ALSO
[`flox-install(1)`](./flox-install.md)
