## Debugging Activation Scripts

Set `FLOX_ACTIVATE_TRACE=1` to trace activation script execution:

```bash
nix build
FLOX_ACTIVATE_TRACE=1 result/bin/flox activate [args]
```

## Generating shell code for tcsh

**Any tcsh code consumed via `eval` of a command substitution MUST double-quote
the backticks.** Use the quoted form, never the bare form:

```tcsh
eval "`flox activate`"   # correct
eval `flox activate`     # WRONG — output is word-split and brace-expanded
```

## Shell hooks and forked subshells

**Shell code that Flox registers to run on a directory change MUST do nothing
in a forked subshell.** zsh's `chpwd_functions` and tcsh's `cwdcmd` alias also
run when a fork of the shell changes directory, and `$$` there still expands
to the parent's PID:

```zsh
x=$(cd .. && pwd)   # the hook must not act here
( cd .. )           # nor here
```

A hook that acts in the fork detaches the parent shell from an activation it is
still using. zsh's own `cd` completion runs a command substitution like the one
above, so no unusual setup is needed to hit it. Never treat `$$` as proof that
the interactive shell itself is running the code; guard new hooks the way
`zsh_hook` and `tcsh_hook` do in `cli/flox-activations/src/hook.rs`.
