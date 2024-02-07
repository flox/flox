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
```

# DESCRIPTION

Create a new empty environment in the current directory.

The name of the environment will be the basename of the current directory
or `default` if the current directory is `$HOME`.
The `--name` flag can be used to give the environment a specific name.

By default the environment will be created in the current directory.
Flox will add a directory `$PWD/.flox` containing all relevant environment 
metadata.
The `--dir` flag can be used to create an environment in another location.

If an environment already exists in the current directory,
or the path specified using `--dir` exists,
an error is returned.

# OPTIONS

## Init Options

`-n <name>`, `--name <name>`
:   What to name the new environment (default: current directory).

`-d <path>`, `--dir <path>`
:   Directory to create the environment in (default: current directory).

```{.include}
./include/general-options.md
```

# See also
[`flox-activate(1)`](./flox-activate.md),
[`flox-install(1)`](./flox-install.md),

