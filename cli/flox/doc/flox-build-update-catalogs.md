---
title: FLOX-BUILD-UPDATE-CATALOGS
section: 1
header: "Flox User Manuals"
...

# NAME

flox-build-update-catalogs - Update catalog lockfile for Nix expression builds

# SYNOPSIS

```
flox [<general-options>] build update-catalogs
     [-d=<path>]
```

# DESCRIPTION

Read the catalog configuration from `.flox/nix-builds.toml` and generate
(or regenerate) the lockfile at `.flox/nix-builds.lock`.

Each catalog entry in `nix-builds.toml` is resolved and pinned:
Nix source-type catalogs are locked with `nix flake prefetch`,
and FloxHub catalogs are locked against the FloxHub API.
The resulting lockfile pins every catalog to a specific revision,
ensuring reproducible builds.

Both `.flox/nix-builds.toml` and `.flox/nix-builds.lock` should be
committed to version control so that all collaborators build against
identical catalog inputs.

This command only works with path (local) environments.
It cannot be used with managed or remote environments.

If no `.flox/nix-builds.toml` file exists, the command prints a warning
with the expected file path and exits without error.

# OPTIONS

```{.include}
./include/dir-environment-options.md
./include/general-options.md
```

# EXAMPLES

## Lock catalog inputs

```
$ flox build update-catalogs
```

# SEE ALSO

[`nix-builds.toml(5)`](./nix-builds.toml.md)
[`flox-build(1)`](./flox-build.md)
[`manifest.toml(5)`](./manifest.toml.md)
