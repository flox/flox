---
title: FLOX-DEVELOP
section: 1
header: "Flox User Manuals"
...


# NAME

flox-develop - Enter a development shell for a Nix expression build


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
machinery of a Nix expression build (a `.nix` file under
`.flox/pkgs/`, see [`flox-build(1)`](./flox-build.md)) loaded and ready
to invoke. This is the equivalent of `nix develop` for that package: a
shell in which to drive the build by hand, reproduce a failure, and
iterate, without running a full `flox build` for every change.

Entering the shell does not require `<package>` to build successfully
first — the shell is built from the package's dependencies, not from a
completed build. This is the primary way to debug a package that
currently fails to build.

`<package>` must be a Nix expression build. Manifest-defined builds
(the `[build]` table in `manifest.toml`) are refused: an unsandboxed
manifest build already runs its script in a shell equivalent to
`flox activate`, so use `flox activate` instead. See
[`flox-build(1)`](./flox-build.md) for manifest builds.

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

> **Note:** `NIX_BUILD_TOP` is a fresh temporary directory the shell
> sets up for you. Running `genericBuild` in the project directory
> instead unpacks the source *into your working tree*, which is almost
> never what you want.

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

This shell approximates the environment in which `flox build` builds
the package. It does not reproduce it exactly, and the differences are
printed each time you enter the shell:

- No build sandbox is applied in the shell. `flox build` runs the build
  under `nix build`, which the Nix daemon may sandbox.
- Your full working tree is visible in the shell, including files git
  does not track. A real build sees only tracked files.
- `$src` was evaluated when you entered the shell and does not follow
  your edits; exit and re-enter to pick them up (see "Working in the
  shell" above). A real build evaluates it fresh every time.
- `$out` and the other output variables point at placeholder paths,
  not at store paths. Nothing installed there is a real build output.
- This shell is interactive and sources `~/.bashrc`, so the tools on
  your `PATH` remain available here, including any Flox environment
  `~/.bashrc` activates; the build inputs come first on `PATH`. A real
  build sees only its own inputs.

## Known limitations

- The shell is always `bash`, regardless of `$FLOX_SHELL` or `$SHELL`,
  and only `~/.bashrc` is sourced — a `~/.zshrc`, `~/.config/fish/`, or
  other shell's startup files are not.

## Garbage collection

The shell's build inputs are protected from garbage collection while
the shell is open: entering it writes a symlink under
`.flox/run/<system>.<package>.develop`, so a concurrent
`nix-collect-garbage` cannot remove them from under a running session.
Re-entering the same package's shell repoints that symlink rather than
adding another, so only the most recent shell for a package is
protected.

## Omitting `<package>`

`flox develop` without a package argument uses the project's only Nix
expression build, as `flox build` does. Unlike `flox build`, it never
resolves to more than one: with several such packages, it lists them so
you can name one; with only manifest builds, it points at
[`flox-activate(1)`](./flox-activate.md); with no builds at all, it
fails the same way `flox build` does on an empty project.

# OPTIONS

`<package>`
:   The package to develop, as defined by its expression file in
    `.flox/pkgs/`. May be omitted if exactly one package in the project
    has a Nix expression build.

`--stability <stability>`
:   Resolve the package's dependencies using a base package set of the
    given stability, as tracked by the catalog server, exactly as
    `--stability` does for [`flox-build(1)`](./flox-build.md). Pass the
    same value to both so the shell and the build agree on their inputs.

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

# EXAMPLES

## Iterating on a failing build

1. Define a Nix expression build and track it with git:

```nix
# file: .flox/pkgs/hello/default.nix
{ stdenv, hello }:

stdenv.mkDerivation {
  pname = "hello";
  version = "1.0";
  src = ./.;
  buildInputs = [ hello ];
  installPhase = "mkdir -p $out; echo hi > $out/hi";
}
```

```console
$ git add .flox/pkgs/hello
```

2. Enter the development shell and run the build phases in a scratch
   directory:

```console
$ flox develop hello
flox [develop: hello] $ mkdir -p "$NIX_BUILD_TOP/work" && cd "$NIX_BUILD_TOP/work"
flox [develop: hello] $ genericBuild
```

3. Edit the unpacked source under `$NIX_BUILD_TOP/work` and re-run a
   single phase. To change the expression instead, exit and run
   `flox develop hello` again:

```console
flox [develop: hello] $ installPhase
flox [develop: hello] $ exit
```

## Running one command in the shell

Print the store path of the source snapshot without entering the shell:

```console
$ flox develop -c 'echo "$src"' hello
```

# SEE ALSO

[`flox-build(1)`](./flox-build.md)
[`flox-build-update-catalogs(1)`](./flox-build-update-catalogs.md)
[`flox-activate(1)`](./flox-activate.md)
[`manifest.toml(5)`](./manifest.toml.md)
