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

Move a path environment to FloxHub or push local changes to a managed
environment to FloxHub.

After pushing, the remote environment can be referred to as `<owner/name>`.

A path environment contains a manifest file and lock file stored locally and
possibly committed to version control.
Pushing the environment moves the manifest and lock file to FloxHub,
and only a reference to the revision of the environment is stored locally.

Once the environment has been pushed, it is called a *managed environment*.
Changes can be made to managed environments locally,
and flox stores those changes in `$XDG_DATA_HOME`.
Those changes are then synced to FloxHub when `flox push` is run.

In the same way as a git repo, local changes to a managed environment may
diverge from the environment on FloxHub if `flox push` is run from a different
host.
Passing `--force` to `flox push` will cause it to overwrite any changes on
FloxHub with local changes to the managed environment.

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
