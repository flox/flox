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
     [--] [<arguments>...]
```

# DESCRIPTION

Run a binary from a Nix package without installing it to an
environment.

`flox run` is designed for one-off invocations.
Instead of requiring the overhead of an environment,
it fetches the required package and executes the binary,
all in one command.

## Binary Lookup

When you run `flox run <binary>`,
Flox queries FloxHub to find which packages provide that binary.
You do not need to know the package name.
For example,
`flox run grep` will find and run the `grep` binary from
the `gnugrep` package without you needing to specify `gnugrep`.

If the binary cannot be found,
Flox will print an error with suggestions and
recommend using `--package` to specify the package directly.

## Disambiguation

If multiple packages provide the same binary
(for example, `vi` is provided by `vim`, `nvi`, and others),
Flox will prompt you to choose which package to use.
Your choice is saved as a preference so that subsequent
invocations of the same binary run silently without
re-prompting.

In non-interactive contexts
(when stdin is not a terminal, such as in pipelines or CI),
Flox will use a saved preference if one exists.
If no preference is saved,
the command will fail with an error listing the packages
that provide the binary and suggesting `--package`.

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
    Flox looks up which package provides this binary via FloxHub.

`-p <package>`, `--package <package>`
:   Specify the package directly,
    bypassing the binary-to-package lookup.
    This is useful when you know the package name
    or when the automatic lookup does not find the right package.
    The choice is saved as a preference
    so that future invocations of the same binary use this package.

`--reselect`
:   Clear the saved preference for this binary and
    re-prompt for disambiguation.
    In non-interactive contexts this will fail with an error
    listing the available packages.

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
$ flox run cowsay "Hello, world\!"
```

Run a binary whose name differs from its package
(`grep` is provided by `gnugrep`):

```
$ flox run --package gnugrep grep -- --color=auto -r "pattern" .
```

Use `--` to pass option-style arguments to the binary:

```
$ flox run curl -- -sL http://example.com
```

Specify the package explicitly:

```
$ flox run --package vim vi
```

Pipe input to a command:

```
$ echo '{"name":"Flox"}' | flox run jq -- '.name'
```

Clear a saved preference and re-select:

```
$ flox run --reselect vi
```

Search for packages that provide a binary:

```
$ flox search --binary rg
```

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md),
[`flox-install(1)`](./flox-install.md),
[`flox-search(1)`](./flox-search.md)
