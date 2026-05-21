---
title: FLOX-DEACTIVATE
section: 1
header: "Flox User Manuals"
...

# NAME

flox-deactivate - deactivate an environment

# SYNOPSIS

```
flox [<general-options>] deactivate
```

# DESCRIPTION

Deactivates an active Flox environment in the current shell,
reversing the changes made by [`flox-activate(1)`](./flox-activate.md).

`flox deactivate` deactivates the innermost (most recently activated)
environment.
When multiple environments are layered,
running `flox deactivate` repeatedly deactivates them one at a time,
innermost first.

Deactivation performs the following steps:

* Runs the `profile.deactivate.${shell}` script from the environment's
  manifest, if defined, allowing the environment to undo modifications
  it made during activation.
* Restores environment variables and shell state to their pre-activation
  values. Only variables that activation set are reverted; variables that
  activation did not touch are left alone.
* Reverts shell customizations: the prompt is restored from its saved
  value (or recomputed from the remaining active environments if any
  remain), command hashing is re-enabled, and any zsh `FPATH` entries,
  completion caches, and precmd hooks installed by activation are
  removed.
* Detaches the current shell from the activation.
  When the last shell detaches,
  services started by the activation are stopped.

## Interactive subshells

Inside a subshell created by an interactive `flox activate`,
`flox deactivate` is equivalent to `exit`:
it exits the subshell and returns to the parent shell,
where any previously active environments are restored automatically.

# OPTIONS

```{.include}
./include/general-options.md
```

# EXAMPLES

Deactivate the innermost active environment:

```
$ flox deactivate
```

Exit a `flox activate` subshell
(equivalent to running `exit`):

```
flox [myenv] $ flox deactivate
$
```

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md)
