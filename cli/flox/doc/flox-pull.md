---
title: FLOX-PULL
section: 1
header: "Flox User Manuals"
...


# NAME

flox-pull - pull environment from FloxHub

# SYNOPSIS

```
flox [<general-options>] pull
     [-d=<path>]
     [-a]
     [-r=<owner>/<name> | <owner>/<name> | [-f]]
```

# DESCRIPTION

Pull an environment from FloxHub and create a local reference to it,
or, if an environment has already been pulled, retrieve any updates.

When pulling an environment for the first time, `-d` specifies the directory
in which to create that environment.
The remote environment is specified in the form `<owner>/<name>`.
It may optionally be preceded by `-r`,
but `-r` is not necessary and is accepted simply for consistency with other
environment commands.

When pulling an environment that has already been pulled, `-d` specifies which
environment to sync.
If `-d` is not specified and the current directory contains an environment, that
environment is synced.
`-f` may only be specified in this case, forceably updating the environment
locally even if there are local changes not reflected in the remote environment.
`<owner>/<name>` may be specified in this case and will replace the environment
with the specified environment.

A remote environment may not support the architecture or operating system of the
local system pulling the environment,
in which case `-a` may be passed to forceably add the current system to the
environment's manifest.
This may create a broken environment that cannot be pushed back to FloxHub until
it is repaired with [`flox-edit(1)`](./flox-edit.md) or
[`flox-remove(1)`](./flox-remove.md).
See [`manifest.toml(1)`](./manifest.toml.md) for more on multi-system
environments.

# OPTIONS

## Pull Options

`-d`, `--dir`
:   Directory to pull an environment into, or directory that contains an
    environment that has already been pulled (default: current directory).

`-a`, `--add-system`
:   Forceably add current system to the environment, even if incompatible.

`-r <owner>/<name>`, `--remote <owner>/<name>`
:   ID of the environment to pull.

`<owner>/<name>`
:   ID of the environment to pull.

`-f`, `--force`
:   Forceably overwrite the local copy of the environment.

```{.include}
./include/general-options.md
```

# SEE ALSO

[`flox-push(1)`](./flox-push.md)
[`flox-edit(1)`](./flox-edit.md)
[`flox-remove(1)`](./flox-remove.md)
[`manifest.toml(1)`](./manifest.toml.md)
