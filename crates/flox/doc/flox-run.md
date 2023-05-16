---
title: FLOX-RUN
section: 1
header: "flox User Manuals"
...


# NAME

flox-run - run app from current project

# SYNOPSIS

flox [ `<general-options>` ] run [ `<run-options>` ] [ -- [ `<command args>` ... ] ]

# DESCRIPTION

Run flake application from the requested package (or "installable").
If not provided `flox` will prompt for you to select from the list of known packages.
`flox run` uses `nix run` under the hood to execute the so-called installable. Which makes it possible to run 
packages from any Nix flake that exposes the `apps` attribute in its outputs.

# EXAMPLES

## Running applications in the current working directory

If `flox run` is called without any arguments, it will ask the user which application they want to use.

```console
$ flox run
? Select a packageapp for flox run  
> flox
  flox-bash
  nix-editor
[↑ to move, enter to select, type to filter]
```

If `flox run` is called with an argument, it will try to run that app instead, without asking for user input.

```
$ flox run cowsay -- 'Moo!'
```

## Running applications from nixpkgs

It is possible to use `flox run`, to run applications from nixpkgs as follows.

```console
$ flox run 'nixpkgs#cowsay' -- 'Moo'
```

Note, if flags have to be passed to the called application, it is done as follows.

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
