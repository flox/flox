## Release 0.3.6 (2023-10-26)

This release addresses several changes, bugs and improvements, including:

- Updated flox CLI prompts to remove mention of GitHub.
- Updated flox CLI so new environments will now
  - use newer nixpkgs for inline packages, and
  - emit fewer warnings regarding unrelated inputs.
- Created `flox show` (currently feature-flagged) and its manpage; `flox search` will now include a hint to 
  use `flox show` for detailed information for a package(s).
- Worked on lots of changes to the flox CLI as we aim for our general availability (1.0) release. As we move
  move forward we are more aggressively making changes to our CLI including:
  - those using the `flox/prerelease` to get upgrades may stop finding upgrades auto install. This is because we are 
    changing core architecture. Please fall back to the installer on our documentation page.
  - we are deprecating the following from the CLI. We need to do this to unblock some dependencies and we also are 
    taking a more holistic design pass based on existing usage. We fully plan on bringing these features 
    back–incorporating feedback. If you depend on them please stop upgrading flox for now and stay on 
    version 0.3.6 or older.
    - `build`
    - `subscribe`
    - `unsubscribe`
    - `channels`
    - `print-dev-env`
    - `shell`
    - `bundle`
    - `flake`
    - `eval`
    - `develop`
    - `publish`
    - `init-package`
    - `bash-passthru`
    - `run`
    - `list`
    - `nix`
    - `import`
    - `export`
    - `envs`
    - `gh`
    - `git`
- Removed the flag `--long` from `flox search`.

We especially want to thank all our github.com/flox/flox contributors and discourse community members for all your valuable feedback!
