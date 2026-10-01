---
title: FLOX-BUILD
section: 1
header: "Flox User Manuals"
...


# NAME

flox-build - Build packages with Flox


# SYNOPSIS

```text
flox [<general-options>] build
     [-d=<path>]
     [--stability <stability>]
     [--override-input <reference>=<flakeref>]...
     [<package>]...
```

# DESCRIPTION

Build the specified `<package>` from the environment in `<path>`,
and the output at `result-<package>` adjacent to the environment.

## Manifest-defined packages

Possible values for `<package>` are all keys under the `build` attribute
in the `manifest.toml`.
If no `<package>` is specified, Flox will attempt to build all packages
that are defined in the `manifest.toml`.

Packages are built by running the script defined in `build.<package>.command`
within a `bash` subshell.
The shell will behave as if `flox activate` was run immediately prior to
running the build script.

### Pure and impure builds

Builds can be performed in a sandbox for the sake of reproducibility.
By default this sandbox is turned off and the build is run in the root of the
repository.
This allows you to perform incremental builds using existing caches and
intermediate build artifacts.
We call this an "impure" build.
With the sandbox disabled, building is similar to running the build script
manually within a shell created by `flox activate`.

Pure builds are run in a temporary directory with the sandbox enabled.
The sandbox can be enabled by setting `build.<package>.sandbox = "pure"`.
Only files tracked by `git` are copied into the temporary build directory.
This ensures that the build doesn't accidentally depend on untracked files.

For this kind of "sandboxed" build, access to untracked files and files outside
of the repository are restricted to provide a reproducible build environment.
Sandboxed builds on Linux additionally are restricted from accessing the
network, but the sandboxing mechanisms on macOS are somewhat limited and thus
pure builds on macOS _will be able to access the network_.

### Referring to other builds

Any build can access the _results_ of other builds (including non-sandboxed
ones) by referring to their name via `${<package>}`.
This allows multi-stage builds.
In the example below, the `app` package depends on the `dep` package
by using `${deps}/node_modules`.

### Build outputs

`flox build` creates a temporary directory for the build script
to output built packages to.
The environment variable `out` is set to this directory,
and the build script is expected to copy or move packages to `$out`.

Upon completion of the build, the build result will be symlinked to
`result-<package>` adjacent to the `.flox` directory that defines the package.

### Metadata

Specifying the `build.<package>.description` and `build.<package>.version`
fields of the build provide extra metadata that can be used by `flox install`,
`flox search`, and `flox show` commands if the build is later published.

The `build.<package>.version` field can be specified in one of the following ways:

1. **as a string**: `version = "0.0.1"`
1. **as read from a file**: `version.file = "<path>"`
1. **as returned by a command**: `version.command = "<cmd> <args>"`

### Catalog imports for Nix expression builds

Nix expression builds (packages defined as `.nix` files under `.flox/pkgs/`)
can depend on packages from FloxHub catalogs.
An expression references a catalog package as `catalogs.<catalog>.<package>`,
where the package receives a `catalogs` argument.
The referenced packages are the ones published to a FloxHub catalog with
`flox publish`.
There is no separate declaration file: the references are discovered by
scanning the expressions themselves.

A project may commit a catalog lock at `.flox/catalog.lock`, created or
refreshed with `flox build update-catalogs`.
A committed lock pins the source of every catalog reference in the project,
so every build of a revision resolves the same inputs; it is used exactly as
found, and is only ever rewritten by `flox build update-catalogs`.
Without a committed lock, `flox build` resolves the references of the
packages being built fresh on every invocation ("lockless" builds);
nothing is written into the project tree.
Catalog references resolve to the latest published versions and are
independent of `--stability`, which selects only the nixpkgs base
package set.

To build against an unpublished revision of a catalog input, such as a
local checkout being iterated on, pass
`--override-input <reference>=<flakeref>`.
The input's source is replaced for that invocation only, as
`nix build --override-input` replaces a flake input;
the committed lock is left unchanged.

`<reference>` names the package as an expression references it, e.g.
`catalogs.myorg.hello`.
Where an expression selects a member of a package
(`catalogs.myorg.toolkit.readVersion`), name the package
(`catalogs.myorg.toolkit`).
The reference must be one the lock pins:
with a committed lock, any catalog input of the project;
without one, a catalog input of the packages being built.

`<flakeref>` is a Nix flake reference with a URL scheme, such as
`git+file:///src/hello` or `github:myorg/hello/my-branch`
(`flake:<name>` for a registry entry).
Any other value is a path, relative to the current directory.
A path inside a git repository is fetched as that repository:
only git-tracked files are fetched, including uncommitted changes to them,
as for the project's own expressions.
A path outside of one is fetched as a `path:` flakeref, which copies the
entire directory into the Nix store.
In either form the source's `.flox` directory is looked for directly
beneath the path given, or beneath the `?dir=<subdir>` of a flakeref.

An override replaces the source of an input and nothing else:

- The input must already be published.
  Without a committed lock the catalog is still asked to resolve it,
  so a package that has never been published cannot be overridden.
- Catalog packages that the overriding source itself references come from
  the project's lock.
  A reference the overriding source adds is not resolved.

# OPTIONS

`<package>`
:   The package(s) to build.
    Possible values are all keys under the `build` attribute
    in the environment's `manifest.toml`.

`--stability <stability>`
:   Perform a nix expression build using a base package set of the given
    stability as tracked by the catalog server.
    A stability (e.g., `"stable"`) identifies a curated nixpkgs revision
    managed by the catalog server.
    When omitted, the base package set is derived from the environment's
    `toplevel` group; if no `toplevel` group exists, the `"stable"`
    stability is used by default.
    An explicit `--stability` value overrides both of these defaults.
    Cannot be used with manifest builds.

`--override-input <reference>=<flakeref>`
:   Fetch the catalog input `<reference>`, as the expression names it
    (e.g. `catalogs.myorg.hello`), from `<flakeref>` for this invocation
    instead of its locked source.
    A value without a URL scheme is a path: the git repository it is in,
    or a `path:` flakeref outside of one.
    May be given more than once, once for each input.
    `.flox/catalog.lock` is not modified.
    Cannot be used unless a Nix expression build is among the packages
    being built.


```{.include}
./include/dir-environment-options.md
./include/general-options.md
```

# EXAMPLES

## Building a simple pure package

1. Add build instructions to the manifest:

```toml
# file: .flox/env/manifest.toml

...
[build]
hello.command = '''
# produce something and move it to $out
mkdir -p $out
echo "hello world" >> $out/hello.txt
'''
description = "Produces a file containing 'hello world'"
version = "0.0.0"
```

2. Build the package and verify its contents:

```console
$ flox build hello
$ ls ./result-hello
hello.txt
$ cat ./result-hello/hello.txt
hello world
```

## Building a simple multi-stage app

Assume a simple `nodejs` project

```text
.
├── .git/
├── package-lock.json
├── package.json
├── public/
├── README.md
├── src/
...
```

1. Initialize a Flox environment

```bash
flox init
```

2. Install dependencies and add build instructions

```toml
# file: .flox/env/manifest.toml
version = 1

[install]
nodejs.pkg-path = "nodejs"
rsync.pkg-path = "rsync"

# install node dependencies using npm
# disable the sandbox to allow access to the network
[build]
deps.command = '''
npm ci
mkdir -p $out
mv node_modules $out/node_modules
'''
deps.sandbox = "off"

# build the application using previously fetched dependencies
app.command = '''
rsync -lr ${deps}/node_modules ./
npm run build
mv dist $out/
'''
```

3. Verify the result

```bash
npx serve result-app
```

# SEE ALSO

[`flox-build-clean(1)`](./flox-build-clean.md)
[`flox-build-import-nixpkgs(1)`](./flox-build-import-nixpkgs.md)
[`flox-build-update-catalogs(1)`](./flox-build-update-catalogs.md)
[`flox-develop(1)`](./flox-develop.md)
[`flox-activate(1)`](./flox-activate.md)
[`manifest.toml(5)`](./manifest.toml.md)
