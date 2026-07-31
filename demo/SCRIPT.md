# Demo: getting Flox out of the invizone

A follow-along script for the prototype on this branch. Roughly 10 minutes.

It answers Ron's question from
[#team-customers](https://flox-dev.slack.com/archives/C08KX3KSB47/p1781043793907199?thread_ts=1781017105.726829&cid=C08KX3KSB47)
— "how do we get out of the invizone" — and Michael Brantley's challenge in the
same thread to show what the auto-activation hook could convey now that it has a
controlling terminal.

The through-line: **Flox has one always-on channel (the prompt) doing three
jobs, and no channel at all for the moment something changes.** That is why it
reads as simultaneously too noisy (DEV-44) and too quiet (this thread). The
prototype splits the budget by frequency.

## Prerequisites

```bash
cd /path/to/flox
git switch daniel/dev-44-activation-visibility-prototype
nix develop -c just build
```

Everything below uses `./target/debug/flox`, so it cannot disturb your installed
Flox. Run it from the repo root in **bash** or **zsh**.

You do **not** need direnv installed. Step 1 explains why.

Set up a scratch environment and a clean config so nothing here touches your
real one:

```bash
export FLOX=$PWD/target/debug/flox
export FLOX_CONFIG_DIR=$(mktemp -d)
DEMO=$(mktemp -d)/acme-api && mkdir -p "$DEMO" && cd "$DEMO"
"$FLOX" init
```

---

## Act 1 — Reproduce the invizone

W&B activates through direnv's `use flox`. The thing to know, which changes the
whole analysis: `use_flox` is

```bash
direnv_load flox activate "${args[@]}" -- "$direnv" dump
```

That is `flox activate -- <CMD>` — **exec-command mode**, not the
`eval "$(flox activate)"` in-place mode the June thread assumed. So we can
reproduce a W&B developer's experience exactly, with no direnv:

```bash
"$FLOX" activate -- bash -c 'echo "PS1 is: [$PS1]"; echo "FLOX_PROMPT_ENVIRONMENTS is: [$FLOX_PROMPT_ENVIRONMENTS]"'
```

**What you should see:** no `flox [...]` in `PS1` — but on this branch, one line
from Flox naming the environment. On `main` there is no such line: this exact
invocation is silent.

**Why the prompt is empty, permanently.** Exec-command mode generates no shell
rc at all, so `set-prompt.bash` — sourced only behind `if [ -t 1 ]` in
`cli/flox-activations/src/gen_rc/bash.rs` — never runs. Independently, direnv
[refuses to export `PS1`](https://github.com/direnv/direnv/wiki/PS1) at all. Two
independent reasons, so:

1. Flipping `hide_default_prompt` to `false`, the first step suggested in the
   thread, would not have changed anything for a single W&B developer.
2. Auto-activation does not engage for them either: the prompt hook is a shell
   function plus a `PROMPT_COMMAND` mutation, and exec mode emits neither.
   **Auto-activate GA, on its own, did nothing for W&B.**

**Why the line appears anyway — the useful surprise.** The June thread assumed
this path had no terminal to speak on. It does: direnv runs the `.envrc` with
`cmd.Stderr = os.Stderr` and captures only stdout, which is exactly why users see
`direnv: loading .envrc` on every activation. Flox's silence here was never
missing terminal access — it was the `InvocationType::Interactive` gate on the
message. Removing that gate reaches W&B **with no migration at all.**

Which reframes the whole problem. W&B developers are not in a silent terminal.
They see `direnv: loading .../.envrc` and a wall of `direnv: export +FLOX_ENV
~PATH ...` on every activation. They have a mental model, and its name is
**direnv**. Flox is doing the work and direnv is getting the attribution.

Confirm the CI half is still clean, since that is the objection this must
survive:

```bash
"$FLOX" activate -- true 2>&1 | cat        # silent: stderr is a pipe
"$FLOX" activate -- true                   # announces: stderr is your terminal
```

Finally, note what is exported. `FLOX_PROMPT_ENVIRONMENTS` is an ordinary
environment variable, so it survives `direnv dump` even though `PS1` cannot.
W&B can have a prompt indicator today with no Flox release at all:

```bash
"$FLOX" activate -- bash -c 'echo "they could render: flox [$FLOX_PROMPT_ENVIRONMENTS]"'
```

Worth offering Brian Lalor regardless of what we ship.

---

## Act 2 — The transition gets a channel

First, the one mode that already spoke — a subshell activation:

```bash
cd "$DEMO"
"$FLOX" activate
```

```
✔ You are now using the environment 'acme-api'
To stop using this environment, run 'flox deactivate'
```

That is **unchanged** behavior from `main`. Flox has always had this message —
it was just gated to `InvocationType::Interactive`
(`cli/flox-activations/src/cli/activate.rs`), the one mode nobody at W&B uses.
Leave the subshell:

```bash
exit
```

Now the in-place mode, which is what a shell rc file and the auto-activation
hook both use. On `main` this prints **nothing at all**:

```bash
eval "$("$FLOX" activate)"
```

**What you should see on this branch:**

```
✔ Activated Flox environment 'acme-api'.
```

One line, and no deactivate hint — because in-place recurs, on every new shell
and every `cd`, and you would read that hint a hundred times a day. Now let the
*hook* do it. The `eval` above registered the prompt hook, so:

```bash
"$FLOX" deactivate
cd / && cd "$DEMO"
```

`cd` back in and the hook activates it for you, with the same one line:

```
✔ Activated Flox environment 'acme-api'.
```

One line. No deactivate hint, because you would read it a hundred times a day.
Note the word **Flox** is in it — this is the channel that earns the brand, not
the prompt.

**The answer to Brantley's objection.** He downvoted "spurious output," and he
was right to. This is not spurious on three counts: it fires only when the set
of active environments actually *changes*; it is what buys us the right to make
the always-on prompt shorter (Act 3); and Flox already ships a default-on
activation notification of exactly this shape — `upgrade_notifications`
(`cli/flox/src/commands/activate.rs`). The argument was never output vs. no
output. It is which output earns its place.

**The answer to James Bayer.** He asked for
`flox environment "foo" activated` / `deactivated`. This is the activated half,
in the existing message channel rather than a new one. The deactivated half is
deliberately **not** in this prototype — see "Not built" below.

Turn it off, and confirm it is genuinely one setting:

```bash
"$FLOX" config --set activation_notifications false
cd / && cd "$DEMO"     # silent
"$FLOX" config --delete activation_notifications
```

`-q` also silences it, which is worth a moment because it was **not** true before
this branch for any mode:

```bash
"$FLOX" -q activate -- true    # silent
```

`flox-activations` has no quiet mode — the verbosity it is handed is clamped to
`max(0)` (`cli/flox/src/commands/activate.rs`), so a negative verbosity never
reached it and the pre-existing subshell message ignored `-q` entirely. Adding
output to a subsystem that cannot be quieted would have been the fair objection
to this whole change, so the decision is made in the `flox` crate, which is the
only place that knows the user's real verbosity.

---

## Act 3 — Spend the savings: the prompt gets shorter

This is [DEV-44](https://linear.app/floxdotdev/issue/DEV-44/shell-prompt-customization-reduce-verbosity-and-improve-clarity),
which you filed in April and which already anticipated this thread: *"we want to
keep some mention of the Flox brand so that it doesn't go entirely invisible
(this is exacerbated by the new auto-activation feature)."*

Michael Stahnke's complaint was the string `flox [metrics-work stahnma/default (local)]`:
the username is redundant, `(local)` is unexplainable, and `flox` costs three
more characters than it needs to.

The full string only appears for FloxHub environments, so the difference is
clearest in the unit test that pins it:

```bash
nix develop -c just unit-tests prompt_detail_narrows_remote_environment_to_its_name
```

It asserts:

| `prompt_detail` | prompt shows |
| --- | --- |
| `full` (today's behavior) | `wandb/core (local)` |
| `name` (**new default**) | `core` |

If you have a FloxHub environment handy, see it live:

```bash
"$FLOX" config --set prompt_detail full
eval "$("$FLOX" activate -r <owner>/<env>)"   # flox [<owner>/<env> (local)]
"$FLOX" deactivate
"$FLOX" config --set prompt_detail name
eval "$("$FLOX" activate -r <owner>/<env>)"   # flox [<env>]
"$FLOX" deactivate
```

`bare_description()` is left alone — it is also what Bash activation errors
quote — so the narrowing happens only on the prompt path
(`ActivateOptions::make_prompt_environments`).

And the `flox` label itself, which already worked in all four shells and was
undocumented until this branch:

```bash
FLOX_PROMPT=f eval "$("$FLOX" activate)"   # f [acme-api] $
"$FLOX" deactivate
```

Because `[vars]` are applied before the prompt is set, an environment can carry
its own label — a real answer for a platform team that wants its own name in
front of its developers:

```toml
[vars]
FLOX_PROMPT = "acme"
```

**The sequencing point.** Brantley's suggested first step — default
`hide_default_prompt` to `false` — is the change that gives *every* user
Stahnke's complaint, because unhiding a `default` environment reinstates exactly
`owner/default (local)`. DEV-44 is therefore a **prerequisite** for that flip,
not a parallel nice-to-have. With `prompt_detail = name` it costs ~6 characters
instead of ~24. This branch does **not** flip it; that is the follow-up once
this lands.

---

## Act 4 — Make the adoption measurable

We cannot currently tell whether anyone is using auto-activation, which means we
cannot tell whether W&B has migrated. The `cli.environment.activate` event
carries `env_detail`, `start_services`, `mode`, `has_includes`,
`lockfile_version`, `manifest_version`, and `shell` — and `mode` is the *dev/run*
activation mode. Nothing in the event records how the activation was invoked.

The hook now says so explicitly:

```bash
cd "$DEMO"
"$FLOX" hook-env --shell bash --shell-pid $$ | grep -o 'activate --auto-activated --dir [^)]*'
```

**What you should see:** the emitted activation carries `--auto-activated`. That
one flag does double duty — it selects the one-line message form in Act 2 and
stamps `auto_activated: true` on the activation event, which is the difference
between guessing at adoption and counting it.

```bash
grep -n "auto_activated" cli/flox-events/src/lib.rs
```

---

## Clean up

```bash
"$FLOX" deactivate 2>/dev/null; cd /; rm -rf "$DEMO" "$FLOX_CONFIG_DIR"
unset FLOX FLOX_CONFIG_DIR DEMO
```

---

## What is not built here, on purpose

- **The deactivation line.** James Bayer asked for it and it is the obvious
  symmetric half, but the hook emits the deactivate script and exits before the
  shell evaluates it, so printing from `hook-env` would announce a teardown that
  has not happened and might fail. Doing it honestly means having the emitted
  script speak, which is a different change. Deliberately deferred.
- **Flipping `hide_default_prompt`.** Prerequisite ordering, see Act 3.
- **The direnv migration.** The largest lever and the largest blast radius. This
  branch makes the destination worth migrating *to*; it does not move anyone.
- **Recording invocation type in telemetry.** `auto_activated` answers the one
  question we have now. The general fix — put the invocation type on the event
  so interactive / in-place / exec are all separable — is a follow-up.
- **The identity gap.** Worth knowing before anyone promises a dashboard: as of
  2026-07-02 only Brian and Jake had FloxHub accounts in the `wandb` org, for
  100+ daily developers. Until CLI authentication is in place, `auth_subject`
  cannot attribute activations to people, so no amount of reporting closes the
  buyer-visibility gap on its own.
