# Repair command-group help

Load this file only after `command-group-help` has failed. Use these patterns
to recommend a correction; the review itself does not authorize editing.

## Existing pattern

The following excerpts come from `services` in
[commands/mod.rs](../../../../cli/flox/src/commands/mod.rs) and
[services/mod.rs](../../../../cli/flox/src/commands/services/mod.rs).
Adapt the actual command and type names rather than copying them literally.

The parent command connects the parser fallback to the hidden help variant:

```rust
Services(
    #[bpaf(
        external(services::services_commands),
        fallback(services::ServicesCommands::Help)
    )]
    services::ServicesCommands,
),
```

Inside the child-command enum, hide the implementation-only help variant:

```rust
/// Prints help information
#[bpaf(command, hide)]
Help,
```

The handler selects the group's help in this match arm, then returns `Ok(())`
after the match:

```rust
ServicesCommands::Help => {
    display_help(Some("services".to_string()));
},
```

Import `display_help` at module scope, following adjacent imports. Preserve
existing handler return conventions and exhaustive match arms, including any
command-name classification required by the dispatcher. Do not copy legacy
telemetry calls from the example or invent new events for this repair.

## Apply the smallest appropriate correction

- Missing fallback: connect the parent child-parser registration to the help
  variant. Do not add defaults to unrelated options.
- Missing help variant or handler: add the hidden variant and route it to the
  shared renderer without running a child operation.
- Wrong help context: select the actual group, keeping existing command names,
  aliases, module placement, visibility attributes, and feature gates intact.
- Invalid input becoming successful help: correct the fallback or catchall
  behavior so unknown children and invalid flags still fail.

`display_help` accepts a single top-level command token. Passing
`Some("parent child".to_string())` supplies one token, not a nested command
path. For a nested group, inspect its parser and existing context-aware help
support; report the limitation rather than prescribing that expression.
The renderer also special-cases `run`, whose catchall consumes `--help`.
Do not replace the helper with a generic reparse or copy that special case
into unrelated commands.

For beta groups, keep registration in the existing beta command enum and the
central dispatcher gate. Do not move a gate into the help handler or expose
a previously hidden command in top-level help.

## Verify the correction

Rerun the applicable inspection and executable checks in
[command-group-help](../checks/command-group-help.md). Confirm contextual
help and success for bare invocation, hidden internal help, retained errors
for invalid input, and unchanged gate behavior. Use the corrected source's
binary; do not report success from a stale executable or a skipped test.
