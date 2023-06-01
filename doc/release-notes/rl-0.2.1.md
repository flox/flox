## Release 0.2.1 (2023-06-01)

This release includes many bug fixes and feature refinements based on feedback from our users, including:
- Updated `search` so you may now search for packages using semantic version ranges:
  Examples: `flox search hello@2`, `flox search 'hello@^2.12'`, `flox search 'hello@^2.12 || 1'`
- Fixed bug which prevented flox destroy from fully deleting environment metadata.
- Updated to issue warning message on MacOS when a github access token is stored in the system keychain.
- Fixed bug that caused users to be prompted to confirm metrics submission twice on the first invocation.
- Fixed bug preventing `flox export` from working for users who hadn't already been using older versions of flox.
- Fixed bug in `flox list` related to when an environment contained a package from a flake
- Updated "dotfile" recommendations to be more applicable to a wider number of (POSIX) shells.
- Added examples to `flox-run` man page.
- Lowered verbosity of nix subprocesses.

We have also been working to overhaul our documentation, floxdocs. The site now has clearer navigation, a new "flox in 5 minutes" tutorial, a dedicated "Install flox" page, a new Concepts section, and a new "Cookbook" section. The intention of the Cookbook is that it's your go-to source of information when you want to know how to do specific tasks with flox, such as setting up a Rust project or running a single command in an environment. There's still lots of work to do, but we're dedicated to making our documentation first class.

We especially want to thank all our github.com/flox/flox contributors and discourse community members for all your valuable feedback!
