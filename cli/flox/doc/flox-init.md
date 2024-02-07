---
title: FLOX-INIT
section: 1
header: "Flox User Manuals"
...


# NAME

flox-init - initialize flox expressions for current project.

# SYNOPSIS

flox [ `<general-options>` ] init [ `<options>` ]

# DESCRIPTION

Create a new empty environment in the current directory.

The name of the environment will be the basename of the current directory
or "default" if the current directory is `$HOME`.
The `--name` flag can be used to give the environment a custom name.

By default the environment will be created in the current directory.
flox will add a directory `$PWD/.flox`,
within which all relevant metadata of the environment will be tracked.
The `--dir` flag can be used to create an environment in another location.

If an environment already exists in the current directory,
or the path specified using `--dir` exists, an error is returned.

# OPTIONS

```{.include}
./include/general-options.md
```

## Init Options

[ \--name `<name>` | -n `<name>` ]
:   Name of the package to be created.
    Queried interactively if controlling TTY is attached

[ \--dir `<path>` | -d `<path>` ]
:   Directory to create the environment in (default: current directory)
