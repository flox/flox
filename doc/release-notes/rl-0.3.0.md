## Release 0.3.0 (2023-08-31)

With this update we are starting to drive towards the flox CLI general availability (1.0) release as described in our earlier [Discourse announcement](https://discourse.floxdev.com/t/upcoming-changes-in-the-flox-cli-0-3-x/787). This first release in the 0.3.x series begins that work with a greater than usual number of user-facing changes and bug fixes, including:

- Updated the CLI to no longer automatically activate "default" when activating other environments.
- Added new `flox auth` command and logic to migrate environment metadata.
    - When first invoking `flox (push|pull)` using this latest version you will be prompted to sign into the new flox CLI (using your GitHub credentials) and migrate your environment metadata to our new "floxHub" cloud offering. As you first sign in you will note that the flox CLI only requires access to your identity and does not require _any_ additional scopes. This migration will be performed only once per device, and you can view your login status on a given device using the new `flox auth status` command.
    - Notably, this metadata migration also allowed us to remove the client-side user `git` configuration that was previously required to store this metadata on GitHub, and similarly prevents user configuration from impacting the use of `git` by the flox CLI.
- Renamed `flox destroy` to `flox delete`; `flox destroy` will be fully deprecated in an upcoming release.
- Updated short help messages for `flox` and some of its subcommands.
- Made the following "feature flagged" changes which only apply after having invoked `flox config --set features.env rust` or when invoked with `FLOX_FEATURES_ENV=rust` in the environment:
    - Updated `flox (init|delete|install|...)` to work against a local `.flox` directory by default
    - Changed the command formerly known as `flox init` that creates a build shell to now be available as `flox init-package` and is hidden from the help page for now.
    - Changed usage of `flox init`: `-e` flag is hidden but still allowed for backwards compatibility, `--name` has been added and is documented but only works using the new env interface.

We especially want to thank all our github.com/flox/flox contributors and discourse community members for all your valuable feedback!
