---
title: FLOX-PULL
section: 1
header: "flox User Manuals"
...


# NAME

flox-pull - pull environment from FloxHub

# SYNOPSIS

```
flox [ <general-options> ] pull
     [-d=<path>]
     [-a]
     [[-f] | -r=<owner/name> | <owner/name>]
```

# DESCRIPTION

Pull an environment from FloxHub and create a managed environment locally
referring to that remote environment,
or, if a managed environment with a reference to a remote environment already
exists, update that environment.

When creating a new managed environment, `-d` specifies the directory in which
to create that environment.
The remote environment is specified in the form `<owner/name>`,
and it may optionally be preceded by `-r`.

When updating a managed environment that already exists, `-d` specifies which
environment to update.
`-f` may only be specified in this case, forceably updating the managed
environment even if there are local changes not reflected in the remote
environment.
`<owner/name>` may not be specified in this case, as the managed environment
already keeps track of its remote environment.

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
:   Directory in which to create a managed environment, or directory that
    already contains a managed environment (default: current directory).

`-a`, `--add-system`
:   Forceably add current system to the environment, even if incompatible.

`-f`, `--force`
:   Forceably overwrite the local copy of the environment.

`-r`, `--remote`
:   ID of the environment to pull.

`<owner/name>`
:   ID of the environment to pull.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO

[`flox-push(1)`](./flox-push.md)
[`flox-edit(1)`](./flox-edit.md)
[`flox-remove(1)`](./flox-remove.md)
[`manifest.toml(1)`](./manifest.toml.md)
