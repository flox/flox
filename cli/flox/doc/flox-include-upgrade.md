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
This includes package upgrades that an included environment has locked without
changing its manifest, for example with `flox upgrade`, for each package group
whose packages all come from that environment.
Such a package group is set to the versions in the included environment's
lockfile once that environment's lock differs from the one the lockfile
records.
Until then, the composing environment keeps the versions it locked itself, for
example with `flox upgrade`.
A package group that mixes packages from several environments, such as the
default `toplevel` group when the composing environment installs packages too,
doesn't get those upgrades.
See [`manifest.toml(5)`](./manifest.toml.md) for more details on how packages
from included environments are locked.
The packages that changed on the current system are listed, and so are the
generations that environments included with `remote` changed between.

If the names of specific included environments are provided, only changes for
those environments will be fetched. If no names are provided, changes will be
fetched for all included environments.

A path environment, or an environment pulled from FloxHub into a directory,
already uses the latest changes to the environments it includes with
`auto-upgrade` enabled, which by default are the path environments it includes
with `dir`, but keeps them out of its lockfile and its generations.
This command saves the changes in use to the lockfile, and gets the latest
contents of other included environments, such as those included with `remote`.
It fetches environments included with `remote` from FloxHub, so it saves their
latest generation even if commands still use an earlier one, which is fetched
in the background.
Package upgrades locked by an environment that an included environment includes
in turn are saved too, unless that included environment's lockfile was written
by an older version of Flox, which doesn't record them.

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
