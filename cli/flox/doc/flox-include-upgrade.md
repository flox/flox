---
title: FLOX-INCLUDE-UPGRADE
section: 1
header: "Flox User Manuals"
...

# NAME

flox-include-upgrade - upgrade an environment with latest changes to its
included environments

# SYNOPSIS

```text
flox [<general-options>] include upgrade
     [-d=<path> | -r=<owner/name>]
     [--check]
     [<included environment>]...
```

# DESCRIPTION

Get the latest contents of included environments and merge them with the
composing environment.

If the names of specific included environments are provided, only changes for
those environments will be fetched. If no names are provided, changes will be
fetched for all included environments.

A path environment already uses the latest changes that the path
environments it includes with `dir` have locked, but keeps them out of its
lockfile.
This command saves them to the lockfile,
and gets the latest contents of other included environments, such as those
included with `remote`.

## Checking the lockfile

`flox include upgrade --check` checks that the lockfile has the latest changes
to the included environments that commands use, without changing anything.
It's meant as an optional check in CI, so that a commit doesn't leave changes
to included environments out of its lockfile.
It succeeds if the lockfile has them, and fails with an error naming the
`flox include upgrade` command to run if:

- included environments have changes that aren't in the lockfile;
- the latest changes to an included environment can't be read or locked,
  for example because its manifest is committed without its lockfile;
- the environment doesn't build with the latest changes on the current system.

It doesn't use or keep the copy of the lockfile in `.flox/cache`, and it builds
the latest changes without replacing the environment that commands use, so it
reports the same wherever it runs.
For an environment that doesn't follow its included environments, there's
nothing to check.

# OPTIONS

`--check`
:   Check that the lockfile has the latest changes to the included
    environments that commands use, without changing anything.
    Fails if it doesn't. Can't be combined with names of included environments.

`<included environment>`
:   Name of included environment to check for changes

```{.include}
./include/environment-options.md
./include/general-options.md
```

# SEE ALSO
[`manifest.toml(5)`](./manifest.toml.md)
