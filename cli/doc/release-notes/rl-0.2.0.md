## Release 0.2.0 (2023-05-17)

This release includes changes related to merging of repositories, bug fixes, and feature refinements based on feedback from our users, including:

- Merged github:flox/flox-bash into the existing github:flox/flox repository as part of the ongoing effort to rewrite flox in rust.
- Updated release processes and flox package versioning to reflect recent repository changes (changes from 
  `flox-A.B.C-rXXX-U.V.W-rYYY` and `flox-U.V.W-rYYY` to 
  `flox-A.B.C-rXXX` and `flox-bash-A.B.C-rXXX` respectively).
- Fixed bug where FLOX_ENV is not a valid path when running `flox activate` in a project environment.
- Fixed bug where running `flox init` could lead to corrupted packages.
- Updated runix to reflect the latest upstream version.
- Added a new top-level --system <system> arg which allows searching catalogs across system types. This change does not 
  remove the existing environment subcommand support and use of the --system flag at the environment subcommand level 
  will override its use at the top level.
- Updated man pages, improved error messages.

We especially want to thank all our github.com/flox/flox contributors and discourse community members for all your valuable feedback!
