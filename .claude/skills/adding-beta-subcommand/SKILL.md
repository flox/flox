---
name: adding-beta-subcommand
description: Use when adding a new beta CLI subcommand to flox, which could also be called an experimental or unstable subcommand. This should be used when adding a command to the `beta` crate that respects the `beta` feature flag.
---

# Adding a new beta subcommand

Beta subcommands are top-level `flox <name>` commands. The
`Commands::Beta` arm in `cli/flox/src/commands/mod.rs` checks
`flox.features.beta` once before dispatching, so individual handlers shouldn't
re-check it.

## When not to use this skill

- Adding a subcommand under an existing top-level command (e.g.
  `flox build subcommand`). This skill is
  only for new top-level `flox <name>` commands gated by
  `features.beta`.
- Promoting a beta command to stable — that's a separate move out of
  the `beta` crate.

Two crates are involved, by design:

- **`cli/beta`** — owns the args struct and `handle()` body. Plain
  bpaf-derived structs; no `command` or `hide` attributes.
- **`cli/flox/src/commands/beta.rs`** — owns the `BetaCommands` enum where
  the command name, `hide`, and dispatch live. This is the reviewed
  surface that enforces beta commands stay hidden from `flox --help`.

## Steps

1. **Create the args + handler** in `cli/beta/src/<snake_name>.rs`.
   Mirror `cli/beta/src/beta_enabled.rs`:
   - `#[derive(Bpaf, Clone, Debug)]` struct holding any options/args.
   - **No `#[bpaf(command(...))]` and no `#[bpaf(hide)]` on the struct.**
     Those attributes live on the variant in the CLI crate.
   - `pub async fn handle(self, flox: Flox) -> Result<()>` with
     `#[instrument(name = "<command-name>", skip_all)]`.
   - Do **not** check `flox.features.beta` — already gated in the CLI.

2. **Register the module** in `cli/beta/src/lib.rs`:
   `pub mod <snake_name>;`

3. **Wire up the command** in `cli/flox/src/commands/beta.rs`:
   - Add a variant to `BetaCommands`. Use `command("<kebab-name>")` and
     **always** include `hide` on the enum variant
     `#[bpaf(hide)]` is not sufficient on its own; without per-variant
     `hide` the subcommand leaks into `flox --help`. (Verify with the
     check in step 4.)

     ```rust
     #[bpaf(command("<kebab-name>"), hide)]
     <CamelName>(#[bpaf(external(<snake_name>::<snake_name>))] <snake_name>::<CamelName>),
     ```

   - Add a match arm to `BetaCommands::handle`:

     ```rust
     BetaCommands::<CamelName>(args) => args.handle(flox).await,
     ```

4. **Verify**, inside a worktree (per the repo `AGENTS.md`) and inside
   `nix develop` (or wrap each command with `nix develop -c` if not
   already in the shell — `cargo` and friends are not on bare PATH):
   - `cargo build -p flox`
   - `./target/debug/flox <kebab-name>` → exits non-zero with:
     ```
     Enable beta features to run this command:
       flox config --set features.beta true
     ```
   - `FLOX_FEATURES_BETA=true ./target/debug/flox <kebab-name>` → runs
     the handler.
   - `./target/debug/flox --help` → the new command must **not** appear.
     If it does, `hide` is missing from the variant.

## Conventions

- Beta commands may freely depend on `flox-rust-sdk`, but when adding beta commands, strive to leave `flox-rust-sdk` code unchanged. Any code in the beta crate doesn't need to be reviewed for stability, but any code changes in other crates will require more thorough review which will make it slower to add the beta command.
- If we need to share code between `beta` and `flox` crates, we may need to
  factor it out to avoid a cyclic dependency.
  Note that this will increase review burden.
- Don't put beta-only logic in `flox` or `flox-rust-sdk`; keep it in
  the `beta` crate.
- Integration tests are not required for beta commands while they
  remain gated.
