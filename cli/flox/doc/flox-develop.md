---
title: FLOX-DEVELOP
section: 1
header: "Flox User Manuals"
...


# NAME

flox-develop - enter a development shell for a Nix expression package


# SYNOPSIS

```text
flox [<general-options>] develop
     [-d=<path>]
     [--stability <stability>]
     [-c=<cmd>]
     [<package>]
```

# DESCRIPTION

Enter an interactive shell with the dependencies and `stdenv` build
machinery of a Nix expression package (a `.nix` file under
`.flox/pkgs/`) loaded and ready to invoke. This is the equivalent of
`nix develop` for a package built with the Nix Expression Feature: it
gives a Nix-literate developer a shell to drive the build by hand,
reproduce a failure, and iterate, without running a full `flox build`
for every change.

Entering the shell does not require `<package>` to build successfully
first — the shell is realised from the package's dependencies, not
from a completed build. This is the primary way to debug a package
that currently fails to build.

`<package>` must be a Nix expression build. Manifest-defined builds
(the `[build]` table in `manifest.toml`) are refused: an unsandboxed
manifest build already runs its script in a shell equivalent to
`flox activate`, so the shell this command would offer is one already
reachable today. See [`flox-build(1)`](./flox-build.md) for manifest
builds.

Like `flox build`, this command requires the environment's `.flox`
directory to be inside a git repository, and the named package's
expression file to be tracked by git.

## Working in the shell

The shell starts in the directory you ran `flox develop` from, and
never changes it. Nothing is unpacked into your working tree
automatically.

There are two distinct edit loops.

**Editing unpacked source.** `$src` is a snapshot in the Nix store
taken when the package was evaluated (see "Differences from a real
build" below); the shell does not read from your working tree for it.
To iterate on the package's own build phases, unpack that snapshot
into a scratch directory and drive the phases by hand:

```console
$ mkdir -p "$NIX_BUILD_TOP/work" && cd "$NIX_BUILD_TOP/work"
$ genericBuild
```

`NIX_BUILD_TOP` is a fresh temporary directory the shell sets up for
you. Running `genericBuild` in the project directory instead unpacks
the source *into your working tree*, which is almost never what you
want.

After `unpackPhase`, edit the unpacked files under
`$NIX_BUILD_TOP/work` and re-run individual phases (`buildPhase`,
`installPhase`, and so on). Each edit takes effect immediately, with
no re-evaluation:

```console
$ vim src/foo.c
$ buildPhase
```

**Editing the expression.** Editing the `.nix` file changes the
derivation, so it needs a new shell: exit and run `flox develop
<package>` again. No commit and no `flox publish` are required — the
evaluation reads the git working tree, including uncommitted changes
to already-tracked files.

The shell provides `stdenv`'s build machinery, including
`genericBuild`. Phase helper functions such as `printPhases` depend on
the package's own `stdenv` and are not guaranteed to be present.

## Differences from a real build

This shell approximates the environment `flox build` actually builds
in. It does not reproduce it exactly, and the differences are printed
on every entry:

- No build sandbox is applied here. `flox build` runs the build under
  `nix build`, which the Nix daemon may sandbox.
- Your working tree is visible here, including files git does not
  track. A real build sees only git-tracked files.
- `$src` is a snapshot in the Nix store, taken when you entered.
  `genericBuild` builds that snapshot, not your working tree; edits
  reach it only when you re-enter (see "Working in the shell" above).
- `$out` and the other output variables point at placeholder paths,
  not at real store paths. Nothing installed there is a real build
  output.
- The host `PATH` stays reachable after the build inputs, and if your
  `~/.bashrc` activates a Flox environment, that environment is on
  `PATH` here too. A real build sees only its own inputs.
- This shell is interactive and sources `~/.bashrc`. The build shell
  does neither.

## Known limitations

- The shell is always `bash`, regardless of `$FLOX_SHELL` or `$SHELL`,
  and only `~/.bashrc` is sourced — a `~/.zshrc`, `~/.config/fish/`, or
  other shell's startup files are not.

## Omitting `<package>`

`flox develop` without a package argument mirrors part of
[`flox-build(1)`](./flox-build.md)'s bare-invocation convention: a
project with a single Nix expression build in `.flox/pkgs/` needs no
name for it, the same as `flox build`. The mirror stops there --
`flox build` with no arguments builds every target it finds, but a
development shell is singular, so `flox develop` never resolves to
more than one.

With exactly one Nix expression build, that build is entered. With
more than one, the command refuses and lists every candidate so you
can name one. With none, and the project defines manifest builds
instead, it refuses and points at
[`flox-activate(1)`](./flox-activate.md). With no builds of either
kind, it refuses with the same "no packages found" guidance
`flox build` gives an empty project.

# OPTIONS

`<package>`
:   The Nix expression package to develop. Corresponds to an
    expression file in `.flox/pkgs/`. May be omitted if the project
    has exactly one Nix expression build.

`--stability <stability>`
:   Resolve the package's dependencies using a base package set of the
    given stability, as tracked by the catalog server. Matches the
    `--stability` flag of [`flox-build(1)`](./flox-build.md): the
    shell must use the same nixpkgs the build would, or the two stop
    agreeing on what a failure means.

`-c`, `--command <cmd>`
:   Run a shell command string in the development shell instead of
    entering it interactively, mirroring the `-c` flag of
    [`flox-activate(1)`](./flox-activate.md).
    The command runs in a non-interactive subshell with the
    development environment sourced first: `~/.bashrc` is not read
    and the entry disclosure is not printed.
    The command's exit status becomes the exit status of
    `flox develop`.

```{.include}
./include/dir-environment-options.md
./include/general-options.md
```

# SEE ALSO

[`flox-build(1)`](./flox-build.md)
[`flox-activate(1)`](./flox-activate.md)
[`manifest.toml(5)`](./manifest.toml.md)
