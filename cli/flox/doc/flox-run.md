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
     [--]
     <command>
     [<arguments>...]
```

# DESCRIPTION

Run a command from a Nix package without installing it to an
environment.

`flox run` is designed for one-off invocations.
Instead of requiring the overhead of an environment,
it fetches the required package and executes the command,
all in one step.

## Specifying the Package

You must specify which package provides the command using
`--package`.
For example,
`flox run --package gnugrep grep` will run the `grep` command from
the `gnugrep` package.

## Passing Arguments

Arguments after the command name are passed to the invoked
command.
Use `--` when passing option-style arguments (e.g. `-s`, `--verbose`)
to the command so they are not interpreted by `flox run`.
Bare arguments such as URLs, filenames, and strings do not
require `--`.

# OPTIONS

## Run Options

`-p <package>`, `--package <package>`
:   Required. The package that provides the command.

`[ -- ] <command> <arguments>`
:   `flox run` runs the provided command and arguments
    from the package given with `--package`.
    The `--` separator is required when invoking commands with
    option-style arguments to prevent them from being interpreted
    by the `flox` command.

```{.include}
./include/general-options.md
```

# EXAMPLES

Run a command with a bare argument (no `--` needed):

```
$ flox run --package cowsay cowsay "Hello, world\!"
```

Run a command whose name differs from its package
(`grep` is provided by `gnugrep`):

```
$ flox run --package gnugrep grep "pattern"
```

Use `--` to pass option-style arguments to the command:

```
$ flox run --package curl -- curl -sL http://example.com
```

Pipe input to a command:

```
$ echo '{"name":"Flox"}' | flox run --package jq -- jq '.name'
```

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md),
[`flox-install(1)`](./flox-install.md),
[`flox-search(1)`](./flox-search.md)
