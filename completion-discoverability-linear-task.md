# Elevate shell-completion discoverability and self-service setup in the flox CLI

**Type:** Improvement / UX
**Priority:** Medium
**Suggested labels:** `cli`, `ux`, `docs`, `dx`
**Area:** `cli/flox` (bpaf), packaging (`pkgs/flox-cli`), docs

---

## Problem

Flox already ships working tab completions for `bash`, `fish`, and `zsh`
(generated from bpaf and bundled by the Nix package), but the feature is
effectively invisible and non-self-serviceable:

1. **Not discoverable in-product.** `flox --help` says nothing about
   completions. The only mention lives in the manpage
   (`cli/flox/doc/flox.md:27-30`, "Command Line Completions"). A user
   discovers completions only by pressing `<Tab>` and noticing it works, or
   by reading `man flox`.

2. **No self-service setup for non-installer / local-dev users.** Completion
   scripts are emitted only inside the Nix package build
   (`pkgs/flox-cli/default.nix:123-127`) via hidden, internal bpaf flags
   (`--bpaf-complete-style-{bash,fish,zsh}`). Someone building flox from
   source (e.g. in `nix develop`) or installing the bare binary has no
   documented, supported way to emit and install the completion script
   themselves. The generating flags are hidden and not meant to be invoked
   by hand.

3. **Not discoverable by agents / programmatically.** Because nothing in the
   visible CLI surface (`--help`, a subcommand) advertises completions, an
   agent driving the CLI cannot learn that completions exist or how to
   enable them. There is no stable, documented command to rely on.

## Goal

Make shell completions **discoverable** (by humans and agents) and
**self-serviceable** (installable without the platform installer), reusing
the completion scripts bpaf already generates. No change to how completions
*work* — only how they are surfaced and installed.

## Deliverables

### 1. `flox completions <SHELL>` subcommand (primary)
- Add a visible subcommand that prints the completion script for the chosen
  shell to stdout, wrapping the existing hidden
  `--bpaf-complete-style-<shell>` machinery.
- Support the shells bpaf currently supports: `bash`, `zsh`, `fish`
  (and `elvish` if low-cost — bpaf supports it).
- Stable, scriptable contract, e.g.:
  - `flox completions bash > ~/.local/share/bash-completion/completions/flox`
  - `flox completions zsh  > "${fpath[1]}/_flox"`
  - `flox completions fish > ~/.config/fish/completions/flox.fish`
- Follow flox CLI output conventions: success/next-step guidance, 80-col
  wrap, sentence-case messages. Print the script to stdout only; put any
  human guidance on stderr so redirection stays clean.

### 2. Surface completions in `flox --help`
- Wire a brief "Command Line Completions" note into the terminal `--help`
  output (today it exists only in the manpage markdown), pointing users at
  `flox completions <SHELL>` and noting that installer-based installs already
  set this up.
- Keep it concise — a short section or footer line, not a wall of text.

### 3. Docs: local-dev / non-installer setup
- Add explicit, copy-pasteable instructions for emitting and sourcing
  completions when building from source or installing the bare binary
  (no platform installer).
- Home for this: `CONTRIBUTING.md` (local dev) and/or `README.md`, plus an
  update to the manpage section (`cli/flox/doc/flox.md`) to reference the new
  `flox completions` command rather than implying completions only ever
  arrive via the installer.

### 4. Agent-friendly surface
- Ensure the capability is machine-discoverable: a stable, documented
  `flox completions` command + its presence in `--help` is enough for an
  agent to find and self-enable completions without out-of-band knowledge.
- Make the per-shell usage hint discoverable via `flox completions --help`.

## Acceptance criteria

- [ ] `flox completions bash|zsh|fish` prints a valid completion script to
      stdout; redirecting it to the conventional path enables completion in a
      fresh shell.
- [ ] `flox completions` and the completions concept appear in `flox --help`
      (or an obvious, documented path from it).
- [ ] `flox completions --help` documents supported shells and the install
      one-liner per shell.
- [ ] CONTRIBUTING/README document how a from-source / non-installer user
      sets up completions; manpage references the new command.
- [ ] Installer-built packages still bundle completions exactly as before
      (no regression to `pkgs/flox-cli/default.nix` behavior).
- [ ] Output obeys flox CLI conventions (stdout = script only; guidance on
      stderr; 80-col wrap; sentence-case).

## Out of scope (track separately)

- **Nushell support.** bpaf 0.9.24 has renderers only for bash/zsh/fish/elvish
  (`render_bash`/`render_zsh`/`render_fish`/`render_simple`); there is no
  `render_nushell` and no `--bpaf-complete-style-nushell`. Adding nushell is
  blocked on upstream bpaf work (or a hand-written flox-side nushell
  completer) and should be a separate issue.

## Technical notes / pointers

- bpaf pinned at `0.9.24` with `features = ["derive", "autocomplete"]`
  (root `Cargo.toml:32`).
- Hidden generator flags invoked by the package build:
  `pkgs/flox-cli/default.nix:123-127` (`installShellCompletion` +
  `--bpaf-complete-style-{bash,fish,zsh}`).
- Manpage section to update: `cli/flox/doc/flox.md:27-30`.
- bpaf completion parse result is already handled in-tree:
  `ParseFailure::Completion` at `cli/flox/src/commands/mod.rs:516`.
- Dynamic value completions (files/dirs/commands) are shell-agnostic and
  unaffected: `SHELL_COMPLETION_{DIR,FILE,COMMAND}` constants at
  `cli/flox/src/commands/mod.rs:92-94`, used via `complete_shell(...)` in
  `activate.rs`, `edit.rs`, `containerize/mod.rs`, `upload.rs`,
  `lock_manifest.rs`, `init/mod.rs`.
- Investigate whether the new subcommand can call bpaf's completion script
  generation directly (preferred) rather than re-shelling into
  `flox --bpaf-complete-style-*`.

## References

- README install methods: `brew install flox`, `.pkg`, `.deb`, `.rpm`, WSL2
  (`README.md:96-104`).
