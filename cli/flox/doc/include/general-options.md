## General Options

`-h`, `--help`
:   Prints help information.

The following options can be passed when running any `flox` subcommand but must
be specified _before_ the subcommand.

`-v`, `--verbose`
:   Increase logging verbosity.
    Diagnostics are off by default. Use `-v` for Flox INFO, `-vv` for DEBUG,
    `-vvv` for TRACE, and `-vvvv` for TRACE from all dependencies.
    Diagnostic output goes to stderr. `FLOX_LOG` overrides these flags.

`-q`, `--quiet`
:   Silence routine notices and diagnostic logs. Errors and command results still print.
    `FLOX_LOG` can enable diagnostics independently; it does not enable routine notices.

