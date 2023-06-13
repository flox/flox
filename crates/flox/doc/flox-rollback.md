---
title: FLOX-ROLLBACK
section: 1
header: "flox User Manuals"
...

# NAME

flox-rollback - rollback to a previous generation of an environment


# SYNOPSIS

flox [ `<general-options>` ] rollback [ `<rollback-options>`]


# DESCRIPTION

Managed environments evolve in atomic generations.
Any change to a managed environment is tracked as a new generation.
By default, `flox activate` will activate the latest generation.

`flox rollback` allows to reset the activated environment to an earlier generation.
Without arguments resets the _default_ environment to the _previous_ generation.
A specific generation number can be provided using the `--to GENERATION` option.


# EXAMPLES

Changes to a managed environment are always made to the _currently active_
environment:

    $ flox create -e demo               # Generation 1 ()
    $ flox install -e demo hello        # Generation 2 ( hello )
    $ flox install -e demo vim          # Generation 3 ( hello, vim )
    $ flox rollback -e demo             # Generation 2 ( hello )
    $ flox install -e demo emacs        # Generation 4 ( hello, emacs )


# OPTIONS

```{.include}
./include/general-options.md
./include/environment-options.md
```


## ROLLBACK OPTIONS

[--to GENERATION]
:   Which generation to rollback to
    If omitted defaults to the previous generation.



# SEE ALSO

[`flox-generations`(1)](./flox-generations.md),
