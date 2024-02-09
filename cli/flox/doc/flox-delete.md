---
title: FLOX-DELETE
section: 1
header: "Flox User Manuals"
...


# NAME

flox-delete - delete an environment

# SYNOPSIS

flox [ `<general-options>` ] delete [ `<options>` ] [ \--origin ] [ \--force ]

# DESCRIPTION

Remove all local data pertaining to an environment.
Does *not* remove “upstream” environment data by default.

Invoke with the `--origin` flag to delete environment data
both upstream and downstream.

Invoke with the `--force` flag to avoid the interactive
confirmation dialog. (Required for non-interactive use.)

# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```

## Delete Options

[ \--origin ]
:   Also delete environment data previously pushed upstream.

[ \--force ]
:   Do not confirm interactively.
