---
title: FLOX-RUN
section: 1
header: "flox User Manuals"
...


# NAME

flox-run - run app from current project

# SYNOPSIS

flox [ `<general-options>` ] run [ `<run-options>` ] [ `<installable>` ] [ -- [ `<command args>` ... ] ]

# DESCRIPTION

Run a flake application from the requested installable.
See the [nix(1)] manual's section on installables for more information.

[nix(1)]: https://nixos.org/manual/nix/stable/command-ref/new-cli/nix.html#installables

# EXAMPLES

## Running applications in the current working directory

If `flox run` is called without any arguments, it will ask the user which installable they want to run.
Note, in this example, it's assumed there's a `flake.nix` in the current directory.

```console
$ flox run
? Select a packageapp for flox run  
> flox
  flox-bash
  nix-editor
[↑ to move, enter to select, type to filter]
```

If `flox run` is called with an argument, it will try to run that installable instead, without asking for user input.
Note, in this example, it's assumed there's a `flake.nix` in the current directory.

```
$ flox run flox -- -- --version
```

## Running applications from nixpkgs

It is possible to use `flox run`, to run packages from nixpkgs as follows.

```console
$ flox run 'nixpkgs#cowsay' -- 'Moo'
```

## Passing flags

Flags can be passed to the called installable as follows.

```console
$ flox run 'nixpkgs#cowsay' -- -- --help
```

# OPTIONS

## RUN OPTIONS

[ -- [ `<command args>` ... ] ]
:   Arguments passed to the application

```{.include}
./include/general-options.md
./include/development-options.md
```

# SEE ALSO

[nix(1)]

[nix(1)]: https://nixos.org/manual/nix/stable/command-ref/new-cli/nix.html

