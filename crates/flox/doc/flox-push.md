---
title: FLOX-PUSH
section: 1
header: "flox User Manuals"
...


# NAME

flox-push -

# SYNOPSIS

flox [ `<general-options>` ] push [ `<options>` ] [ \--force ] [ ( -m | \--main ) ]

# DESCRIPTION

(`git`) Push metadata to the environment's `floxmeta` repository.
With this mechanism environments can be pushed and pulled between machines
and within teams just as you would any project managed with `git`.

With the `(-m|\--main)` argument `flox (push|pull)` will operate on the
"floxmain" branch, pushing user metadata to the upstream repository.
Cannot be used in conjunction with the `-e|\--environment` flag.

With the `--force` argument flox will forceably overwrite either the
upstream or local copy of the environment based on having invoked
`push` or `pull`, respectively.


# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```

## Push Options

[ (-m | \--main ) ]
:   operate on the "floxmain" branch,
    push user metadata to the upstream repository.
    Cannot be used in conjunction with the `-e|--environment` flag.


[ \--force ]
:   forceably overwrite the upstream copy of the environment

# SEE ALSO

-   *flox-pull(1)*
