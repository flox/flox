---
title: FLOX-PULL
section: 1
header: "flox User Manuals"
...


# NAME

flox-pull -

# SYNOPSIS

flox [ `<general-options>` ] pull [ `<options>` ] [ \--force ]

# DESCRIPTION

(`git`) Push or pull metadata to the environment's `floxmeta` repository,
and in the `pull` case also proceed to render the environment.
With this mechanism environments can be pushed and pulled between machines
and within teams just as you would any project managed with `git`.

With the `--force` argument flox will forceably overwrite either the
upstream or local copy of the environment based on having invoked
`push` or `pull`, respectively.

With the `--no-render` argument `flox pull` will fetch and incorporate
the latest metadata from upstream but will not actually render or create
links to environments in the store. (Flox internal use only.)

# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```

## Pull Options

[ \--force ]
:   forceably overwrite the uppstream copy of the environment

[ \--no-render ]
:   do not render or create links to environments in the store
    (Flox internal use only.)


# SEE ALSO

-   *flox-push(1)*
