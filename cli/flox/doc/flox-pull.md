---
title: FLOX-PULL
section: 1
header: "flox User Manuals"
...


# NAME

flox-pull -

# SYNOPSIS

flox [ `<general-options>` ] pull [ `<options>` ] [ \--force ] [ ( -m | \--main) ]

# DESCRIPTION

(`git`) Push or pull metadata to the environment's `floxmeta` repository,
and in the `pull` case also proceed to render the environment.
With this mechanism environments can be pushed and pulled between machines
and within teams just as you would any project managed with `git`.

With the `--force` argument flox will forceably overwrite either the
upstream or local copy of the environment based on having invoked
`push` or `pull`, respectively.

With the `(-m|\--main)` argument `flox (push|pull)` will operate on the
"floxmain" branch, pulling user metadata from the upstream repository.
Cannot be used in conjunction with the `-e|\--environment` flag.

With the `--no-render` argument `flox pull` will fetch and incorporate
the latest metadata from upstream but will not actually render or create
links to environments in the store. (Flox internal use only.)

# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```

## Pull Options

[ (-m | \--main ) ]
:   operate on the "floxmain" branch,
    pull user metadata from the upstrea repository.
    Cannot be used in conjunction with the `-e|--environment` flag.

[ \--force ]
:   forceably overwrite the upstream copy of the environment

[ \--no-render ]
:   do not render or create links to environments in the store
    (Flox internal use only.)


# SEE ALSO

-   *flox-push(1)*
