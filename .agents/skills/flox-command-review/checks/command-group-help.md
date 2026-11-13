# command-group-help

Load this check only for a command group without an intentional default, or
when the command's bare-invocation behavior needs investigation.

## Applicability

A command group selects child commands, such as `flox services start`.
Review its behavior when invoked without a child, including when an existing
group gains or changes a child. Rust enums that merely organize top-level
help headings are not user-facing command groups.

Preserve deliberate default actions. For example, `ConfigArgs` defaults to
`ConfigArgs::List` in [general.rs](../../../../cli/flox/src/commands/general.rs).
Leaf commands and confirmed intentional defaults are `N/A` for this rule.
If the intended default is unclear, investigate the handler and documentation;
report `NEEDS JUDGMENT` rather than replacing the behavior with help.

## Expected behavior and precedent

A bare applicable group displays its own contextual help, including available
child commands, and succeeds. It must not fall through to generic top-level
guidance, launch a child operation, or require the user to guess a subcommand.
Unknown children and invalid options must remain errors.

The established implementation connects three parts:

- The parent parser supplies a fallback for the child-command parser.
- The child enum has a hidden `Help` variant, absent from the command list.
- The handler routes that variant to the shared contextual-help renderer and
  returns success without executing a child operation.

Compare the `UseCommands::Services` registration and `display_help` renderer
in [commands/mod.rs](../../../../cli/flox/src/commands/mod.rs) with the
`ServicesCommands` enum and handler in
[services/mod.rs](../../../../cli/flox/src/commands/services/mod.rs).
The `generations` group provides a second precedent in
[generations/mod.rs](../../../../cli/flox/src/commands/generations/mod.rs).
Reconfirm these symbols in the reviewed source; do not rely on frozen line
numbers or require identical spelling when equivalent behavior is established.

## Inspection checklist

1. Trace the command's registration, child parser, and help handler together.
   A `Help` variant alone does not establish a working fallback. An option's
   unrelated default does not satisfy this check either.
2. Confirm the help belongs to this group, contains its real child commands,
   and hides the internal help variant. Check for existing catchall parsers
   or special help handling before applying the usual convention.
3. **Only for beta or otherwise gated groups:** inspect the gate in the
   dispatcher. For beta commands, see `Commands::Beta` in
   [commands/mod.rs](../../../../cli/flox/src/commands/mod.rs) and registration
   in [commands/beta.rs](../../../../cli/flox/src/commands/beta.rs).
   With the gate enabled, bare invocation should show contextual help. With
   it disabled, preserve the existing gate response. Preserve top-level
   visibility rules. Explicit `--help` can be handled by the parser before
   dispatch; do not require it to follow the bare-invocation gate path.
4. When executable validation is available, use a binary built from the
   reviewed source, with isolated configuration and telemetry disabled.
   Follow the checkout's dev-shell instructions. Check bare group invocation,
   explicit group `--help`, an unknown child, and an invalid option. Inspect
   stdout, stderr, exit status, and the actual command list; no child operation
   should run. For gated groups also check bare invocation with the gate on
   and off, and top-level help visibility. Do not pin output wording, stream,
   or layout merely because one precedent uses it. If the binary is absent
   or stale, report source evidence and unverified runtime behavior instead.

## Failure handling

Record the concrete failure and its evidence first. Only then read
[the repair guide](../references/command-group-help-fix.md) for examples and
appropriate corrections. A generic missing-subcommand error for an applicable
group is a failure of this convention. An intentional default, a disabled
feature gate, or lack of runtime tooling is not evidence of that failure.
