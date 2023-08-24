## Release 0.2.3 (2023-06-29)

This release includes many bug fixes and feature refinements based on feedback from our users, including:
- Updated default flox environment template in support of new environment initialization features.
- Improved failure handling for `flox activate` and detecting non-interactive environments when running in CI.
- Added `--file | -f` option to `flox edit` which allows replacing the environment declaration with the contents of a file (or to read from stdin if `-` is passed).
- Environments named with the prefix `_testing_` are now reserved for usage by the `flox` test suite.
Users will see a warning ( not an error ) if they create an environment with these names.
Please be advised that while you can use environments with these names, doing so will cause them to be deleted if you build `flox` from source and/or run our test suite directly without sufficient isolation.

We especially want to thank all our github.com/flox/flox contributors and discourse community members for all your valuable feedback!
