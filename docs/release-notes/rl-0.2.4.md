## Release 0.2.4 (2023-07-13)

This release includes many bug fixes and feature refinements based on feedback from our users, including:
- Implemented `flox wipe-history` command, required for nix garbage collection. 
- Updated flox to use nix 2.15.1.
- Updated flox subcommands to accept flake-related options for the building of flox environment packages.
- Updated `flox publish` so it can be used non-interactively by passing --upload-to '' --download-from '' ( empty strings ).
- Updated `flox publish` to allow the publishing of packages from branches other than the default branch.
- Updated `flox publish` so you can now run publish without needing access to builtfilter and building builtfilter is no longer required at runtime.
- Fixed bug in the parsing of command arguments passed to `flox activate`.
- Fixed bug related to installing "unfree" packages from the flox managed nixpkgs catalog.
- Fixed bug that prevented installation of packages using nix flake references such as `github:<OWNER>/<REPO>/<REF>`.
- Fixed broken links in the flox README.

We especially want to thank all our github.com/flox/flox contributors and discourse community members for all your valuable feedback!
