# Flox `zdotdir`

When spawning a new shell with `flox activate`, it's essential that flox
be in a position to configure the environment _last_, after all system and
user-specific configuration "rc" files have been processed, simply to prevent
these scripts from perturbing the flox environment.

Unlike `bash`, `zsh` does not support `--rcfile`, `--norc` or `--no-profile`
options for manipulating the user and system-specific initialization, but it
does offer a `ZDOTDIR` environment variable that can be used to specify an
entirely new set of "system" configuration files to be used at startup.
Flox uses this mechanism to point to files in this directory for all flox
activations involving `zsh`, and our goal in creating this collection is for
each of these scripts to first perform all of the expected "normal" processing,
followed by the flox-specific initialization.

## Implementation

The `flox` script is responsible for setting `ZDOTDIR` to point to this
directory as it invokes `zsh --no-globalrcs`, but not before preserving the
original value of `ZDOTDIR` (if defined) in `FLOX_ORIG_ZDOTDIR`. The `.zshrc`
script then uses this environment variable to restore `ZDOTDIR` to its original
value prior to sourcing the system and user-provided configuration files.
