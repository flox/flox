---
title: FLOX-INIT
section: 1
header: "Flox User Manuals"
...


# NAME

flox-init - initialize a Flox environment

# SYNOPSIS

```
flox [<general-options>] init
     [-n <name>]
     [-d <path>]
     [-b]
     [--auto-setup]
     [--no-auto-setup]

flox [<general-options>] init
     [-r <owner/name>]
     [-b]
```

# DESCRIPTION

Create a new empty environment in the current directory or on FloxHub.

The name of the environment will be the basename of the current directory
or `default` if the current directory is `$HOME`.
The `--name` flag can be used to give the environment a specific name.

By default, the environment will be created in the current directory.
Flox will add a directory `$PWD/.flox` containing all relevant environment
metadata.
The `--dir` flag can be used to create an environment in another location.

If an environment already exists in the current directory,
or the path specified using `--dir` exists,
an error is returned.

`init` will try to detect languages being used in the containing directory,
and it will prompt with suggestions for packages or activation scripts to be
added to the environment.
These suggestions can be taken without prompting by passing `--auto-setup`.
The suggestions can be accepted but then edited using `flox edit`.
Currently, suggestions are made for Python and Nodejs.

With `--reference` Flox will create a FloxHub environment which can
subsequently be used by other commands using the `--reference` flag.
When creating FloxHub environments Flox _will not_ run any setup hooks based
on the current directory.


# OPTIONS

## Common Init Options

`--bare`
:   Set up the environment with the emptiest possible manifest.

## Options to initialize a local environment

`-n <name>`, `--name <name>`
:   What to name the new environment (default: current directory).

`-d <path>`, `--dir <path>`
:   Directory to create the environment in (default: current directory).

`--auto-setup`
:   Apply Flox recommendations for the environment based on what languages are
    being used in the containing directory.

`--no-auto-setup`
:   Don't auto-detect language support for a project or make suggestions.

## Options to initialize an environment on FloxHub

`-r <owner>/<name>`, `--reference <owner>/<name>`
:   The environment reference for the FloxHub Environment to be created.
    User needs write access to `<owner>`, and an environment with the same
    `<name>` must not already exist.


```{.include}
./include/general-options.md
```

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md),
[`flox-install(1)`](./flox-install.md),
