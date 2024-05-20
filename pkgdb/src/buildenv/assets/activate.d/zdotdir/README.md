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
original value of `ZDOTDIR` (if defined) in `FLOX_ORIG_ZDOTDIR`.

The zsh initialization sequence as described in the `zsh(1)` man page is:

1. source `/etc/zshenv` followed by `$ZDOTDIR/.zshenv`
2. if a *login shell*, source `/etc/zprofile` followed by `$ZDOTDIR/.zprofile`
3. if an *interactive shell*, source `/etc/zshrc` followed by `$ZDOTDIR/.zshrc`
4. if a *login shell*, source `/etc/zlogin` followed by `$ZDOTDIR/.zlogin`

Our goal is to source our own `$FLOX_ZSH_INIT_SCRIPT` last, so we must apply
conditional logic to figure out which of the first, third or fourth case above
will be the last to be sourced, then append the sourcing of our script to that.
This logic can be found at the end of each of the `.{zshenv,zshrc,zlogin}`
scripts.

The `ZDOTDIR` environment variable is restored to its original value by the
`$FLOX_ZSH_INIT_SCRIPT` as it is sourced.
