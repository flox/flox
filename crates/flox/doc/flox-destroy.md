---
title: FLOX-DESTROY
section: 1
header: "flox User Manuals"
...


# NAME

flox-destroy - destroy an environment

# SYNOPSIS

flox [ `<general-options>` ] destroy [ `<options>` ] [ \--origin ] [ \--force ]

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

## Destroy Options

[ \--origin ]
:   Also delete environment data previously pushed upstream.

[ \--force ]
:   Do not confirm interactively.
