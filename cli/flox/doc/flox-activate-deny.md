---
title: FLOX-ACTIVATE-DENY
section: 1
header: "Flox User Manuals"
...


# NAME

flox-activate-deny - deny auto-activation for an environment

# SYNOPSIS

```text
flox [<general-options>] activate deny
     [-d=<path>]
```

# DESCRIPTION

```{.include}
./include/auto-activate-experimental.md
```

Prevents the selected environment from being auto-activated.

Once an environment is denied,
the Flox prompt hook skips it silently when you enter a directory containing it,
and you are no longer prompted for it.
The preference is stored in the user config file under
`auto_activate_environments`,
keyed by the absolute path of the directory that contains the `.flox`
directory.

Denying an environment does not deactivate it if it is already active;
it only prevents future auto-activation.
Run [`flox-deactivate(1)`](./flox-deactivate.md) to leave an environment that is
currently active.

By default `flox activate deny` targets the environment in the current
directory.
Use `--dir` to target an environment in another directory.

To allow an environment to be auto-activated again, run
[`flox-activate-allow(1)`](./flox-activate-allow.md).

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

Deny auto-activation for the environment in the current directory:

```bash
flox activate deny
```

Stop being prompted for an environment in another directory:

```bash
flox activate deny -d /path/to/project
```

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md),
[`flox-activate-allow(1)`](./flox-activate-allow.md),
[`flox-config(1)`](./flox-config.md)
