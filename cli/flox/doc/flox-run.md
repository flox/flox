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
     -p=<package>
     <binary>
     [--] [<arguments>...]
```

# DESCRIPTION

Run a binary from a Nix package without installing it to an
environment.

`flox run` is designed for one-off invocations.
Instead of requiring the overhead of an environment,
it fetches the required package and executes the binary,
all in one command.

## Specifying the Package

You must specify which package provides the binary using
`--package`.
For example,
`flox run --package gnugrep grep` will run the `grep` binary from
the `gnugrep` package.

## Passing Arguments

Arguments after the binary name are passed to the invoked
binary.
Use `--` when passing option-style arguments (e.g. `-s`, `--verbose`)
to the binary so they are not interpreted by `flox run`.
Bare arguments such as URLs, filenames, and strings do not
require `--`.

# OPTIONS

## Run Options

`<binary>`
:   Required. The name of the binary to run.
    `flox run` runs this binary from the package given with
    `--package`.

`-p <package>`, `--package <package>`
:   Required. The package that provides the binary.

`[--] <arguments>`
:   Arguments passed to the invoked binary.
    The `--` separator is optional for bare arguments but
    required when passing option-style arguments (e.g. `-f`,
    `--verbose`) to prevent them from being interpreted by
    `flox run`.

```{.include}
./include/general-options.md
```

# EXAMPLES

Run a command with a bare argument (no `--` needed):

```
$ flox run --package cowsay cowsay "Hello, world\!"
```

Run a binary whose name differs from its package
(`grep` is provided by `gnugrep`):

```
$ flox run --package gnugrep grep -- --color=auto -r "pattern" .
```

Use `--` to pass option-style arguments to the binary:

```
$ flox run --package curl curl -- -sL http://example.com
```

Pipe input to a command:

```
$ echo '{"name":"Flox"}' | flox run --package jq jq -- '.name'
```

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md),
[`flox-install(1)`](./flox-install.md),
[`flox-search(1)`](./flox-search.md)
