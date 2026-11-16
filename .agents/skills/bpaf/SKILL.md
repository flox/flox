---
name: bpaf
description: Use when adding, changing, or reviewing CLI commands parsed with bpaf.
---

# bpaf

For commands with subcommands whose bare invocation does nothing useful,
use `fallback_to_usage` on `#[bpaf(command, ...)]` and provide a `help`
subcommand.
