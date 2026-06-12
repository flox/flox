---
title: FLOX-ACTIVATE-ALLOW
section: 1
header: "Flox User Manuals"
...

# NAME

flox-activate-allow - allow auto-activation for an environment

# SYNOPSIS

```
flox [<general-options>] activate [-d=<path> | -r=<owner>/<name>] allow
```

# DESCRIPTION

Marks an environment as allowed for auto-activation. When an environment is
allowed, Flox will automatically activate it when entering the directory
containing that environment, if the `auto_activate` config option is set
to `allowed`.

This command updates the user configuration file to store the preference
for the specified environment.

# OPTIONS

## Environment Selection

`-d <path>`, `--dir <path>`
:   Path containing a .flox/ directory

`-r <owner>/<name>`, `--reference <owner>/<name>`, `--ref <owner>/<name>`
:   A FloxHub environment

`-D`, `--default`
:   Shorthand for `-r <current_user>/default`

If no environment selection option is provided, Flox will use the environment
in the current directory.

```{.include}
./include/general-options.md
```

# EXAMPLES

Allow auto-activation for the environment in the current directory:
```
$ flox activate allow
```

Allow auto-activation for a specific directory:
```
$ flox activate allow --dir ~/projects/myapp
```

# SEE ALSO

[`flox-activate-deny(1)`](./flox-activate-deny.md),
[`flox-config(1)`](./flox-config.md)
