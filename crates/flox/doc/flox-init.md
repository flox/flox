---
title: FLOX-INIT
section: 1
header: "flox User Manuals"
...


# NAME

flox-init - initialize flox expressions for current project.

# SYNOPSIS

flox [ `<general-options>` ] init [ `<options>` ]
# DESCRIPTION

Add a new package using a template.

Given a `<template>` creates a new package definition in `PROJECT_ROOT/<name>`.

An existing package called `<name>` will raise an error.

If `<template>` or `<name>` are unspecified and a controlling TTY is present,
flox will query them using interactive dialogs.

In non interactive shells the command will fail without a `<name>` and default
to a generic builder for `<template>`.

# OPTIONS

```{.include}
./include/general-options.md
```

## Init Options

[ \--name `<name>` | -n `<name>` ]
:   Name of the package to be created.
    Queried interactively if controlling TTY is attached

[ \--template `<template>` | -t `<template>` ]
:   Template to create new package definition from.
