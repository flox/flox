---
title: FLOX-EDIT
section: 1
header: "flox User Manuals"
...


# NAME

flox-edit - edit declarative format of an environment

# SYNOPSIS

flox [ `<general-options>` ] edit [ `<options>` ]

# DESCRIPTION

Edit environment declaratively. Has the effect of creating the
environment if it does not exist.

# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```

## Edit Options

[ (\--file|-f) `<file>` ]
:   Replace environment declaration with that in `<file>`.
    If `<file>` is `-`, reads from stdin.
