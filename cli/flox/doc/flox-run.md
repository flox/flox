---
title: FLOX-RUN
section: 1
header: "Flox User Manuals"
...

# NAME

flox-run - run a command from a Flox Catalog package

# SYNOPSIS

```
flox [<general-options>] run
     -p <package>
     [ -- ] <command> [<arguments>]
```

# DESCRIPTION

Run a command from a Flox Catalog package without creating an environment.

`flox run` is designed for one-off invocations.
It resolves the requested package from the Flox Catalog,
downloads its store paths,
and executes the command directly —
no `flox init`, `flox install`, or environment cleanup needed.

## Specifying the Package

The `-p`/`--package` flag is required and names the package explicitly.
For example:

```
$ flox run -p gnugrep grep "pattern" file.txt
```

The package name is a plain Flox Catalog attribute path
(e.g. `curl`, `python3Packages.requests`).

## Flags Before and After the Command

`flox run` uses POSIX stop-at-first-positional parsing:
flags before the command name belong to `flox run`;
flags after the command name belong to the command.

```
flox run -p curl curl http://example.com  # -p belongs to flox run
flox run curl -p curl                     # ❌ error: -p is curl's, flox has no package
```

Use `--` to pass option-style arguments to the command:

```
$ flox run -p curl -- curl -sL http://example.com
```

The `--` separator is required only when the command name or its
first argument starts with `-`.
Bare arguments such as URLs, filenames, and strings do not require `--`.

**`--version` caveat:**
Flox intercepts `--version` from the full argument list before parsing.
To pass `--version` to the invoked command, use `--` first:

```
$ flox run -p hello -- hello --version   # ✅ shows hello's version
$ flox run -p hello hello --version      # ❌ shows flox's version instead
```

## Command Lookup

`flox run` looks up the command strictly inside the resolved package's
output directories: `bin/` first, then `sbin/`
(`bin/` wins if both contain the command).
The file must be a regular file with an executable bit set.

There is no fallback to the caller's `PATH`.
If the command is not found in the package,
`flox run` exits with an error.

## Exec Semantics

`flox run` replaces the flox process with the invoked command via `exec`.
The caller's PID becomes the command,
signals go directly to it,
and the shell sees the command's exit code.
Stdin, stdout, and stderr are inherited unmodified.

## Caching

Downloaded store paths are registered as GC roots under
`$FLOX_CACHE_DIR/run-gc-roots/`.
Repeated invocations of the same package skip the download step.

# OPTIONS

## Run Options

`-p <package>`, `--package <package>`
:   Required. The Flox Catalog package that provides the command.
    Accepts plain package names only (e.g. `curl`, `ripgrep`).
    Version constraints (`@`), output selectors (`^`), and custom
    catalogs (`/`) are not supported in this release.

`[ -- ] <command> [<arguments>]`
:   The command to run and any arguments to pass to it.
    Use `--` when the command name or first argument starts with `-`.

```{.include}
./include/general-options.md
```

# EXAMPLES

Run a command with a bare argument (no `--` needed):

```
$ flox run -p cowsay cowsay "Hello, Flox!"
```

Run a command whose name differs from the package name:

```
$ flox run -p binutils readelf -a /bin/ls
```

Use `--` to pass option-style arguments to the command:

```
$ flox run -p curl -- curl -sL http://example.com
```

Pipe input to a command:

```
$ echo '{"name":"Flox"}' | flox run -p jq jq '.name'
```

Show the command's own help:

```
$ flox run -p hello -- hello --help
```

# LIMITATIONS

This release (phase 1) requires the `-p`/`--package` flag.
The following features are not yet supported and will be available
in a future release:

- Defaulting the package to the command name (`flox run readelf`)
- Version constraints: `flox run -p curl@8.0 curl …`
- Output selectors: `flox run -p foo^dev …`
- Custom catalogs: `flox run -p mycatalog/vim vim …`
- Executable-to-package lookup and disambiguation

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md),
[`flox-install(1)`](./flox-install.md),
[`flox-search(1)`](./flox-search.md)
