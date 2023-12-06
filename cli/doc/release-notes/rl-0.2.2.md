## Release 0.2.2 (2023-06-15)

This release includes many bug fixes and feature refinements based on feedback from our users, including:
- Updated `flox search` to group packages in a more compact manner when -v,--verbose is used, and aligns columns when -v is not used.
- Renamed the option `flox search -v,--verbose` to `-l,--long`. The `-v,--verbose` flag is deprecated and will be removed in the future.
- Fixed obscure bug affecting the correctness of `flox publish` when working with impure inputs.
- Fixed bug with `flox channels` command returning only the first in a list of channels.
- Improved integration testing of our CLI.
- Made improvements in the flake/project resolution improving responsiveness and performance.
- Added manpage for `flox rollback`.
- Updated `flox init` to remove the option to create a git repository with flox and instead give an explanatory error message.
- Updated `flox generations` command to sort results numerically.
- Fixed but in `flox activate --system <SYSTEM>`.
- Fixed bug that inadvertently modified the user's git config.
- Fixed bug which affected the ability to upgrade packages (both flakes and catalog packages) with `flox upgrade`.
- Added support for `dash` shell.
- The recommended invocation of `flox activate` has been changed in order to support shells such as `dash` and `zsh`. The recommended way to activate environments from a script is now:
    ```
    # For a "named environment":
    eval "$( flox activate -e <NAME> )"
    # For the default environment:
    eval "$( flox activate )"
    ```
- Currently `flox develop` will default to launching a `bash` shell. If you're running a different shell (such as `zsh`), you can use `eval "$( flox print-dev-env '.#<NAME>' )"` to incorporate the project's build environment into your current running shell. In a future release, we'll improve `flox develop` to drop you into your shell by default.

We especially want to thank all our github.com/flox/flox contributors and discourse community members for all your valuable feedback!
