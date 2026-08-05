# Demo: activation visibility

A follow-along script for this branch. ~5 minutes, bash or zsh, with `flox`
pointing at a build of this branch.

## Setup

```bash
export FLOX_CONFIG_DIR=$(mktemp -d)
DEMO=$(mktemp -d)/acme-api && mkdir -p "$DEMO" && cd "$DEMO"
flox init
```

## 1 — An activation notification in every mode

Prompt frameworks like starship regenerate `PS1` on every prompt, after Flox's
one-time prepend, so the `flox [...]` indicator never survives them. Stand one
in with two lines and activate:

```bash
bash --norc --noprofile -i
# in the new shell:
PS1="BASE> "
_fake_starship() { PS1="STARSHIP> "; }   # stands in for starship's precmd
PROMPT_COMMAND="_fake_starship"
eval "$(flox activate -d "$DEMO")"
```

```
✔ You are now using the environment 'acme-api'
```

Starship still owns the prompt, but the transition is no longer silent — and
the notification is written to stderr, so no framework can erase it.

Auto-activation announces the same way. The `eval` above registered the prompt
hook, so a second environment activates on `cd` alone:

```bash
DEMO2=$(mktemp -d)/billing && mkdir -p "$DEMO2"
flox init --dir "$DEMO2" && flox activate allow --dir "$DEMO2"
cd "$DEMO2"
```

```
✔ You are now using the environment 'billing'
```

The message is identical in every mode — subshell (`flox activate`), in-place
(`eval`), auto-activation (`cd`), and command mode (`flox activate -- <CMD>`).
Only the subshell adds `To stop using this environment, run 'flox deactivate'`,
since that is the one mode you have to type your way out of. Apart from
subshells, the notification only fires when stderr is a terminal, so CI and
scripts stay clean:

```bash
flox activate -d "$DEMO" -- true 2>&1 | cat   # silent: stderr is a pipe
flox activate -d "$DEMO" -- true              # announces: stderr is a tty
```

## 2 — Turning it off

One setting, and `-q` for one-offs:

```bash
flox config --set activation_notifications false
cd / && cd "$DEMO2"                # silent
flox config --delete activation_notifications

flox -q activate -- true           # silent, in every mode
```

## 3 — A shorter prompt by default

`prompt_detail` now defaults to `name`, so a FloxHub environment shows as
`flox [core]` instead of `flox [wandb/core (local)]`. Compare with any FloxHub
environment (a fresh shell per comparison):

```bash
flox config --set prompt_detail full
bash --norc --noprofile -i -c 'PS1="BASE> "; eval "$(flox activate -r <owner>/<env>)"; echo "$PS1"'
# → flox [<owner>/<env> (local)] BASE>

flox config --delete prompt_detail
bash --norc --noprofile -i -c 'PS1="BASE> "; eval "$(flox activate -r <owner>/<env>)"; echo "$PS1"'
# → flox [<env>] BASE>
```

That savings pays for more visibility: `hide_default_prompt` now defaults to
`false`, so an active `default` environment appears in the prompt — `default`
costs 8 columns at `name` detail, not the 24 of `owner/default (local)`. Flox
is now visible in every shell where the default environment is active:

```
flox [acme-api default] BASE>
```

The `flox` label itself is `$FLOX_PROMPT`, settable per environment:

```toml
[vars]
FLOX_PROMPT = "acme"
```

## Clean up

```bash
exit
cd /; rm -rf "$DEMO" "$DEMO2" "$FLOX_CONFIG_DIR"
unset FLOX_CONFIG_DIR DEMO DEMO2
```
