## Release 1.0.2 (2024-03-28)

This release addresses several bugs and adds new improvements, including:

 - Flox handles expired FloxHub tokens more gracefully and prompts you to login again.
 - We've improved how shell hooks work in Flox--giving you more precise control and making hooks work more consistently across platforms: **There is now a `[profile]` section of the manifest.toml** that is sourced directly into your shell and enables the use of shell aliases (among other things!). The `profile.common` script is sourced into every shell (good for writing scripts that work in every target environment) as well as shell specific profiles (`profile.zsh` and `profile.bash`) for handling shell-specific behavior. In addition, **there is a new `hook.on-activate` script** that runs in a non-interactive Bash sub-shell after sourcing the profile scripts. This allows you to run a script in a consistent environment where you don't have to worry about shell compatibility. Read more in `man manifest.toml` or our [documentation](https://flox.dev/docs).
 - `flox init` now detects Python requirements and suggests an initial environment with installed software and a `profile.common` script. This will happen when using a requirements.txt in your project directory or a pyproject.toml with poetry.
 - Error messages improved: Added the `pull --force` suggestion to diverging branch error and improved the `flox activate` error when there is no environment.
 - Made several long messages from Flox shorter, making them easier to read. Along the same lines, as an homage to the punch card (and default terminal window sizes), `--help` commands wrap to 80 characters.
 - Removed a not-yet-functional `--file` option hint from the default manifest template and unimplemented commands.
 - We began to implement anonymous tracing telemetry to better support Flox without having to rely on our users manually opening issues.

We especially want to thank all our github.com/flox/flox contributors and
[slack community members](https://go.flox.dev/slack) for all your valuable feedback!


