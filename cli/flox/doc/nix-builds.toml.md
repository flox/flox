---
title: NIX-BUILDS.TOML
section: 5
header: "Flox User Manuals"
...

# NAME

nix-builds.toml - catalog configuration for Nix expression builds

# SYNOPSIS

The `nix-builds.toml` file declares external catalogs that are made
available to Nix expression builds within a Flox environment.
It lives at `.flox/nix-builds.toml` alongside the environment manifest.

# DESCRIPTION

When a Flox environment uses Nix expression builds (packages defined
as `.nix` files under `.flox/pkgs/`), those expressions can depend on
packages provided by external catalogs.
The `nix-builds.toml` file declares which catalogs are available and
where they come from.

Running `flox build update-catalogs` resolves every catalog entry and
writes the pinned result to `.flox/nix-builds.lock`.
Both files should be committed to version control.

## `version`

Required.
The configuration format version.
Currently the only supported value is `1`.

```toml
version = 1
```

## `[catalogs.<name>]`

Each section under `catalogs` declares a single catalog.
The `<name>` becomes the key used to reference the catalog in Nix
expressions: a package `foo` in catalog `mycatalog` is accessed as
`catalogs.mycatalog.foo`.

A catalog can be specified in one of three forms.

### Structured Nix source type

Provide a `type` field naming a Nix source type together with
additional fields appropriate to that type:

```toml
[catalogs.mycatalog]
type = "git"
url = "https://github.com/org/repo"
ref = "main"
```

The supported types and their fields are documented in the
[Nix manual under *Source types*](https://nix.dev/manual/nix/latest/language/builtins.html#source-types).

### URL string

As a shorthand for the structured form, provide a single `url`
field containing a Nix source reference:

```toml
[catalogs.mycatalog]
url = "git+https://github.com/org/repo"
```

The URL follows Nix source reference syntax and may include query
parameters such as `?ref=<branch>` or `?rev=<commit>`.

### FloxHub catalog

Set `type` to `"floxhub"` to pull packages from a catalog published
on FloxHub:

```toml
[catalogs.mycatalog]
type = "floxhub"
```

The catalog name must match a catalog identifier registered on FloxHub.

## Lockfile

Running `flox build update-catalogs` produces `.flox/nix-builds.lock`,
a JSON file that pins every catalog to a specific resolved state.
The lockfile is consumed at build time; it must be present and up to
date before running `flox build` on packages that reference catalogs.

## Using catalogs in Nix expressions

Nix expressions under `.flox/pkgs/` receive a `catalogs` argument.
Each catalog declared in `nix-builds.toml` appears as an attribute set
keyed by `<name>`:

```nix
# .flox/pkgs/hello.nix
{ catalogs }:
catalogs.mycatalog.some-package
```

# EXAMPLES

## Declare a Git catalog

```toml
version = 1

[catalogs.mylib]
url = "git+https://github.com/org/mylib"
```

## Declare a catalog with a pinned branch

```toml
version = 1

[catalogs.mylib]
type = "git"
url = "https://github.com/org/mylib"
ref = "release-2.0"
```

## Declare a FloxHub catalog

```toml
version = 1

[catalogs.myorg]
type = "floxhub"
```

## Use a catalog in a package expression

```nix
# .flox/pkgs/app.nix
{ catalogs }:
catalogs.myorg.build-tool
```

# SEE ALSO

[`flox-build-update-catalogs(1)`](./flox-build-update-catalogs.md)
[`flox-build(1)`](./flox-build.md)
[`manifest.toml(5)`](./manifest.toml.md)
