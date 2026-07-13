---
title: FLOX-RUN
section: 1
header: "Flox User Manuals"
...

# NAME

flox-run - run a command from a Flox Catalog package

# SYNOPSIS

```text
flox [<general-options>] run
     [-p <package>]
     [--reselect]
     <command> [--] [<arguments>]
```

# DESCRIPTION

Run a command from a Flox Catalog package without creating an environment.

`flox run` is designed for one-off invocations.
It finds the package that provides the requested command,
downloads its store paths,
and executes the command directly —
no `flox init`, `flox install`, or environment cleanup needed.

## Package Selection

You do not need to know which package provides a command.
When invoked without `-p`/`--package`,
`flox run` queries the FloxHub command-to-package index
to find the packages whose outputs contain the command:

```bash
$ flox run readelf -- -a /bin/ls   # readelf is provided by binutils
```

The package is selected in this order:

1. **Explicit `--package`.**
   The named package is used directly, bypassing the lookup,
   and is saved as the preference for this command.
2. **Saved preference.**
   A previously saved choice for this command is used silently.
   Use `--reselect` to clear it and choose again.
3. **Index lookup.**
   The command name is looked up in the command-to-package index:
   - If exactly one package provides the command, it is used silently.
   - If several packages provide the command and one of them is named
     exactly like the command, that package is used silently.
   - Otherwise an interactive prompt asks which package to use,
     and the choice is saved as the preference for this command.

If no package provides the command, `flox run` exits with an error
suggesting `--package` as an escape hatch.

Use [`flox search --command <command>`](./flox-search.md) to inspect
which packages provide a command without running anything.

## Saved Preferences

Choices made explicitly — via `--package` or the disambiguation prompt —
are saved as user-level preferences,
so subsequent invocations of the same command run silently
without re-prompting.

Preferences are stored in the user configuration under
`command_preferences` and can be inspected with `flox config`
or managed directly:

```bash
$ flox config --set command_preferences.vi vim
```

Pass `--reselect` to clear the saved preference for a command
and choose again:

```bash
$ flox run --reselect vi
```

## Non-Interactive Invocations

When there is no terminal to prompt on
(piped input or output, scripts, CI),
`flox run` never prompts and never hangs:

- A saved preference (or an exact package name match) is used silently.
- Otherwise the command fails fast with an error that lists the
  candidate packages inline and suggests `--package`:

```text
Multiple packages provide 'vi' and no preference is saved.
Packages with this command: vim, neovim, vimer
Use 'flox run --package <PACKAGE> vi' to specify a package.
```

## Flags Before and After the Command

`flox run` uses POSIX stop-at-first-positional parsing:
flags before the command name belong to `flox run`;
everything after the command name is passed to the command verbatim.

Always use `--` between the command name and its arguments when
the arguments contain flags:

```bash
$ flox run curl -- -sL http://example.com   # -sL goes to curl
```

A single `--` immediately after the command name is treated as the
separator and is not passed to the command.
Use `--` before the command name if the name itself starts with `-`.

**`--version` caveat:**
Flox intercepts a bare `--version` from the full argument list before
parsing, so the separator is required for `--version` to reach the
command:

```bash
$ flox run hello -- --version   # ✅ shows hello's version
$ flox run hello --version      # ❌ shows flox's version instead
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

`<command>`
:   The command to run.
    Without `--package`, the command name is looked up in the
    FloxHub command-to-package index to find the providing package.

`-p <package>`, `--package <package>`
:   The Flox Catalog package that provides the command,
    bypassing the command-to-package lookup.
    Accepts a package name (e.g. `curl`, `python3Packages.requests`),
    optionally with a version constraint (e.g. `curl@8.0`),
    or a custom catalog package (e.g. `mycatalog/vim`).
    The choice is saved as the preference for this command.

`--reselect`
:   Clear the saved package preference for the command and choose again.
    Requires an interactive terminal.

`[--] [<arguments>]`
:   Arguments passed to the command verbatim.
    A single `--` between the command name and its arguments is
    treated as a separator and not forwarded;
    use it whenever the arguments contain flags.
    Use `--` before the command name if the name itself starts with `-`.

```{.include}
./include/general-options.md
```

# EXAMPLES

Run a command (the package is found via the index):

```bash
$ flox run hello
```

Run a command whose name differs from the package name:

```bash
$ flox run readelf -- -a /bin/ls
```

Choose between several packages that provide the same command
(the choice is saved for next time):

```bash
$ flox run vi
```

Clear a saved preference and choose again:

```bash
$ flox run --reselect vi
```

Specify the package explicitly, with a version constraint:

```bash
$ flox run -p curl@8.0 curl -- -sL http://example.com
```

Pipe input to a command:

```bash
$ echo '{"name":"Flox"}' | flox run jq '.name'
```

Show the command's own help or version:

```bash
$ flox run hello -- --help
$ flox run hello -- --version
```

# LIMITATIONS

- Output selectors (`flox run -p foo^dev …`) are not supported.
- Saved preferences are user-level, not per-project.

## Binary cache requirement

`flox run` fetches packages by store path using substitution only.
It does not evaluate Nix expressions or build packages from source
unless the binary cache does not have the package.
A package must have a pre-built binary available in the Nix binary
cache, or be buildable from source.

Packages that require building from source — including those with
unfree licenses that are not pre-cached — cannot be substituted
directly. `flox run` will build such packages from source automatically,
displaying a progress indicator while the build runs.
Press Ctrl-C to cancel if you do not want to wait.
Built packages are stored in a temporary GC root that is removed when
the command exits.
Use `flox install` to add a package to a persistent environment instead.

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md),
[`flox-install(1)`](./flox-install.md),
[`flox-search(1)`](./flox-search.md),
[`flox-config(1)`](./flox-config.md)
