---
title: FLOX-PUSH
section: 1
header: "flox User Manuals"
...


# NAME

flox-push - send environment to FloxHub

# SYNOPSIS

```
flox [ <general-options> ] push
     [-d=<path>]
     [-o=<owner>]
     [-f]
```

# DESCRIPTION

Move an environment's manifest to FloxHub or sync local changes to an
environment to FloxHub.

After pushing, the remote environment can be referred to as `<owner/name>`.

A path environment contains a manifest file and lock file stored locally and
possibly committed to version control.
Pushing the environment moves the manifest and lock file to FloxHub,
and only a reference to the revision of the environment is stored locally.

Once the environment has been pushed, it can be used directly with the
`--remote` option,
or it can be used and edited locally before syncing with `flox push`.

In the same way as a git repo, local changes to an environment that has been
pushed may diverge from the environment on FloxHub if `flox push` is run from a
different host.
Passing `--force` to `flox push` will cause it to overwrite any changes on
FloxHub with local changes to the environment.

# OPTIONS

## Push Options

`-d`, `--dir`
:   Directory to push the environment from (default: current directory).

`-o`, `--owner`
:   Owner to push push environment to (default: current user).

`-f`, `--force`
:   forceably overwrite the remote copy of the environment.

```{.include}
./include/general-options.md
```

# SEE ALSO

[`flox-pull(1)`](./flox-pull.md)
