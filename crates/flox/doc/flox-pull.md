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

(`git`) pull metadata to the environment's `floxmeta` repository.
With this mechanism environments can be pushed and pulled between machines
and within teams just as you would any project managed with `git`.

With the `--force` argument flox will forceably overwrite either the
upstream or local copy of the environment based on having invoked
`push` or `pull`, respectively.


# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```

## Pull Options

[ \--force ]
:   forceably overwrite the uppstream copy of the environment


# SEE ALSO

-   *flox-push(1)*
