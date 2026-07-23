---
title: FLOX-ACTIVATE-ALLOW
section: 1
header: "Flox User Manuals"
...


# NAME

flox-activate-allow - allow auto-activation for an environment

# SYNOPSIS

```text
flox [<general-options>] activate allow
     [-d=<path>]
```

# DESCRIPTION

Permits the selected environment to be auto-activated.

Once an environment is allowed,
the Flox prompt hook activates it automatically whenever you enter a directory
containing it,
without prompting (and deactivates it when you leave).
The preference is stored in the user config file under
`auto_activate_environments`,
keyed by the absolute path of the directory that contains the `.flox`
directory.

By default `flox activate allow` targets the environment in the current
directory.
Use `--dir` to target an environment in another directory.

To stop an environment from being auto-activated, run
[`flox-activate-deny(1)`](./flox-activate-deny.md).

See the *AUTO-ACTIVATION* section of [`flox-activate(1)`](./flox-activate.md)
for the full picture, including the consent prompt and how to enable the
feature.

# OPTIONS

`-d`, `--dir`
:   Path containing a .flox/ directory (defaults to the current directory).

```{.include}
./include/general-options.md
```

# EXAMPLES

Allow auto-activation for the environment in the current directory:

```bash
flox activate allow
```

Allow auto-activation for an environment in another directory:

```bash
flox activate allow -d /path/to/project
```

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md),
[`flox-activate-deny(1)`](./flox-activate-deny.md),
[`flox-config(1)`](./flox-config.md)
