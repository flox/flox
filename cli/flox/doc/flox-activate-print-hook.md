---
title: FLOX-ACTIVATE-PRINT-HOOK
section: 1
header: "Flox User Manuals"
...


# NAME

flox-activate-print-hook - print the Flox prompt hook without activating an
environment

# SYNOPSIS

```text
flox [<general-options>] activate print-hook
```

# DESCRIPTION

Prints shell code that adds the Flox auto-activation prompt hook.
Source the output in an interactive shell to enable auto-activation without
activating a default environment.

To add the prompt hook to your shell,
add the command to your shell's startup file:

Bash (`~/.bashrc`) and Zsh (`~/.zshrc`):

```bash
eval "$(flox activate print-hook)"
```

Fish (`~/.config/fish/config.fish`):

```fish
flox activate print-hook | source
```

Tcsh (`~/.tcshrc`):

```tcsh
eval "`flox activate print-hook`"
```

Any in-place activation, such as `eval "$(flox activate -D)"`,
already adds the hook, so `print-hook` is not needed as well.
The command fails if `disable_hook = true`.

See the *AUTO-ACTIVATION* section of [`flox-activate(1)`](./flox-activate.md)
for how the hook decides which environments to activate.

# OPTIONS

```{.include}
./include/general-options.md
```

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md),
[`flox-activate-allow(1)`](./flox-activate-allow.md),
[`flox-activate-deny(1)`](./flox-activate-deny.md),
[`flox-config(1)`](./flox-config.md)
