---
title: FLOX-BUILD-IMPORT-NIXPKGS
section: 1
header: "Flox User Manuals"
...

# NAME

flox-build-import-nixpkgs - Import package definition from nixpkgs

# SYNOPSIS

```
flox [<general-options>] build import-nixpkgs
     [-d=<path>]
     [--force]
     <installable>
```

# DESCRIPTION

Import a package definition from nixpkgs for use in the environment.
This command copies the source code of a package from nixpkgs into the
environment's `.flox/pkgs/` directory, allowing you to modify and build
the package locally.

The package definition is imported as a Nix expression file at
`.flox/pkgs/<package>/default.nix`, where `<package>` is the attribute
path of the package (e.g., `hello` for `nixpkgs#hello`).

This is useful when you need to:
- Modify a package's build process
- Apply patches or customizations
- Debug package issues
- Create variants of existing packages

## Installable format

The `<installable>` parameter can be specified in one of the following formats:

1. **Attribute path only**: `hello` (defaults to `nixpkgs#hello`)
2. **Flake reference with attribute**: `nixpkgs#hello`
3. **Full flake reference**: `github:nixos/nixpkgs#hello`

# OPTIONS

`<installable>`
:   The package to import from nixpkgs.
    Can be specified as an attribute path (e.g., `hello`) or as a flake
    reference with attribute path (e.g., `nixpkgs#hello`).

`--force`
:   Overwrite existing package file if it already exists.
    Without this flag, the command will fail if the package file
    already exists in the environment.

```{.include}
./include/dir-environment-options.md
./include/general-options.md
```

# EXAMPLES

## Import a simple package

Import the `hello` package from nixpkgs:

```bash
$ flox build import-nixpkgs hello
```

This creates `.flox/pkgs/hello/default.nix` with the package definition.

## Import from a specific nixpkgs revision

Import a package from a specific nixpkgs revision:

```bash
$ flox build import-nixpkgs github:nixos/nixpkgs/nixos-23.11#hello
```

## Overwrite an existing package

Force import a package, overwriting any existing definition:

```bash
$ flox build import-nixpkgs --force hello
```

## Import a complex package

Import a package with a nested attribute path:

```bash
$ flox build import-nixpkgs python310Packages.requests
```

This creates `.flox/pkgs/python310Packages/requests/default.nix`.

# NOTES

- This command only works with local environments (not managed or remote environments)
- The imported package definition is a snapshot of the source code at the time of import
- You can modify the imported package definition and build it using `flox build`
- The package will be available in the environment's build context

# SEE ALSO

[`flox-build(1)`](./flox-build.md)
[`flox-build-clean(1)`](./flox-build-clean.md)
[`manifest.toml(5)`](./manifest.toml.md)
