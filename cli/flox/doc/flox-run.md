---
title: FLOX-RUN
section: 1
header: "Flox User Manuals"
...


# NAME

flox-run - run a command from a Flox Catalog package

# SYNOPSIS

```text
flox [<general options>] run
     [-p=<package[@version]>]
     <executable> [-- | <arguments>...]
```

# DESCRIPTION

Run an executable from a package in the Flox Catalog
without creating a persistent environment.
The package is resolved, its store paths are downloaded,
and the executable is run via `exec` — replacing the
`flox` process entirely.

The package name defaults to the executable name.
For example, `flox run curl http://example.com` resolves
the `curl` package and runs the `curl` executable with
the given arguments.
Use `-p` when the package and executable names differ
(`flox run -p binutils readelf -a /bin/ls`).

## Argument Parsing

`flox run` uses POSIXLY_CORRECT (getopt-style) argument parsing:
argument processing stops at the first positional argument
(the executable name).
All arguments that follow the executable are forwarded to
the executable verbatim.
This means `flox run curl -v http://example.com` passes
`-v` and the URL to `curl` without requiring `--`.

Use `--` before the executable name only when the executable
name itself starts with `-`:
`flox run -- -weirdname`.

## Version Constraints

Append `@<version>` to the executable name or package spec
to request a specific version:

- `flox run curl@8.0` — resolves `curl` at version 8.0
- `flox run -p python@3.11 python3` — resolves `python`
  at version 3.11 and runs `python3`

When `@version` is used without `-p`, the executable name
is derived by stripping the version suffix
(`flox run curl@8.0` looks for the `curl` executable).

## Custom Catalogs

Use `-p <catalog>/<pkg>` to install from a custom catalog:

```
flox run -p mycatalog/vim vi
```

## Process Semantics

`flox run` replaces the `flox` process via `exec`.
This provides:

- **Clean exit codes** — the shell sees the target's exit code.
- **Signal handling** — signals go directly to the target process.
- **Piped stdin** — stdin is forwarded to the target unchanged.

## Non-interactive Use

`flox run` never prompts the user.
In scripts, CI pipelines, or when stdin is not a terminal,
it either succeeds immediately or fails with a clear error.

# OPTIONS

`<executable>`
:   The name of the executable to run.
    If `-p` is not specified, this also determines the package
    to resolve (`flox run curl` resolves the `curl` package).
    Append `@<version>` to request a specific version
    (`flox run curl@8.0`).

`-p`, `--package`
:   Override the package to resolve.
    Accepts the same syntax as `flox install`:
    a package attribute path, optionally with `@<version>`
    or a `<catalog>/<pkg>` prefix for custom catalogs.
    The executable name is always the positional argument.

`[--] <arguments>...`
:   Arguments forwarded verbatim to the executable.
    `--` is required only when the executable name itself
    starts with `-`.

```{.include}
./include/general-options.md
```

# EXAMPLES

Run `cowsay`:

```console
$ flox run cowsay "Hello, world"
 _____________
< Hello, world >
 -------------
        \   ^__^
         \  (oo)\_______
```

Run `readelf` (executable in `binutils` package):

```console
$ flox run -p binutils readelf -a /bin/ls
```

Run a specific version of `curl`:

```console
$ flox run curl@8.0 --version
curl 8.0.1 ...
```

Forward stdin through `cat`:

```console
$ echo test | flox run cat
test
```

Exit code is forwarded:

```console
$ flox run false; echo $?
1
```

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md),
[`flox-install(1)`](./flox-install.md)
