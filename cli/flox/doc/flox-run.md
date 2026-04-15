---
title: FLOX-RUN
section: 1
header: "Flox User Manuals"
...

# NAME

flox-run - run a command without installing it

# SYNOPSIS

```
flox [<general-options>] run
     [-p=<package>]
     [--reselect]
     <binary>
     [<arguments>...]
```

# DESCRIPTION

Run a binary from a Nix package without installing it to an environment.

`flox run` is designed for one-off invocations where creating an
environment would be unnecessary overhead.
It creates a temporary environment behind the scenes,
installs the required package,
executes the binary,
and cleans up when the command exits.

## Binary Lookup

When you run `flox run <binary>`,
Flox queries FloxHub to find which packages provide that binary.
You do not need to know the package name.
For example,
`flox run readelf` will find and run the `readelf` binary from
the `binutils` package without you needing to specify `binutils`.

If the binary cannot be found,
Flox will print an error with suggestions and
recommend using `--package` to specify the package directly.

## Disambiguation

If multiple packages provide the same binary
(for example, `vi` is provided by `vim`, `nvi`, and others),
Flox will prompt you to choose which package to use.
Your choice is cached so that subsequent invocations of the same
binary run silently without re-prompting.

In non-interactive contexts
(when stdin is not a terminal, such as in pipelines or CI),
Flox will use a previously cached choice if one exists.
If no cached choice is available,
the command will fail with a helpful error listing
the available packages and suggesting `--package`.

## Passing Arguments

Everything after the binary name is passed directly to the binary.
Flox options such as `--package` and `--reselect` must appear
*before* the binary name.

A `--` separator is accepted but not required.
It can be useful for readability or when you want to be explicit
about where `flox run` options end and binary arguments begin.

# OPTIONS

## Run Options

`<binary>`
:   The name of the binary to run.
    Flox looks up which package provides this binary via FloxHub.

`-p <package>`, `--package <package>`
:   Specify the package directly,
    bypassing the binary-to-package lookup.
    This is useful when you know the package name
    or when the automatic lookup does not find the right package.
    The choice is saved to the cache
    so that future invocations of the same binary use this package.

`--reselect`
:   Clear the cached package choice for this binary and
    re-prompt for disambiguation.
    In non-interactive contexts this will fail with an error
    listing the available packages.

`<arguments>`
:   All arguments after the binary name are passed directly to the
    invoked binary.
    A `--` separator is accepted but not required.

```{.include}
./include/general-options.md
```

# EXAMPLES

Run a command from a package:

```
$ flox run cowsay "Hello, world!"
```

Run a binary whose name differs from its package
(`readelf` is provided by `binutils`):

```
$ flox run readelf --version
```

Specify the package explicitly:

```
$ flox run --package vim vi
```

Pipe input to a command:

```
$ echo '{"name":"Flox"}' | flox run jq '.name'
```

Clear a cached choice and re-select:

```
$ flox run --reselect vi
```

The `--` separator is optional but still accepted:

```
$ flox run curl -- -sL http://example.com
```

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md),
[`flox-install(1)`](./flox-install.md),
[`flox-search(1)`](./flox-search.md)
