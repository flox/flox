---
title: FLOX-BUILD
section: 1
header: "flox User Manuals"
...


# NAME

flox-build - run app from current project

# SYNOPSIS

flox [ `<general-options>` ] run [ `<options>` ] [ -- [ `<command args>` ... ] ]

# DESCRIPTION

Run flake application from the requested package (or "installable").
If not provided `flox` will prompt for you to select from the list of known packages.

# OPTIONS

```{.include}
./include/general-options.md
./include/development-options.md
```

## Run Options

[ -- [ `<command args>` ... ] ]
:   Arguments passed to the application
