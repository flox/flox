## General Options

Many flox commands wrap Nix commands of the same name,
and will correspondingly pass on options and arguments
directly to the underlying `nix` invocation.
For more information on the options supported by specific Nix commands
please invoke `flox nix <command> help`.

The following options are used specifically by `flox`
and must be specified _before_ the `<command>` argument.

-v, \--verbose
:   Verbose mode. Invoke multiple times for increasing detail.

\--debug
:   Debug mode. Invoke multiple times for increasing detail.

-V, \--version
:   Print `flox` version.

\--prefix
:   Print `flox` installation prefix / Nix store path.
    (flox internal use only.)
