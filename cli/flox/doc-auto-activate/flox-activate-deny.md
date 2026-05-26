---
title: FLOX-ACTIVATE-DENY
section: 1
header: "Flox User Manuals"
...

# NAME

flox-activate-deny - deny auto-activation for an environment

# SYNOPSIS

```
flox [<general-options>] activate [-d=<path> | -r=<owner>/<name>] deny
```

# DESCRIPTION

Marks an environment as denied for auto-activation. When an environment is
denied, Flox will not automatically activate it when entering the directory
containing that environment, regardless of the `auto_activate` config option
setting.

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

Deny auto-activation for the environment in the current directory:
```
$ flox activate deny
```

Deny auto-activation for a specific directory:
```
$ flox activate deny --dir ~/projects/untrusted
```

# SEE ALSO

[`flox-activate-allow(1)`](./flox-activate-allow.md),
[`flox-config(1)`](./flox-config.md)
