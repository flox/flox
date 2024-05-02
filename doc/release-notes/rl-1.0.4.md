## Release 1.0.4 (2024-04-23)

###  Environments

- Paths with spaces are now supported in general. (#1277)
- Flox will now notify the user on stdout if a new version is available.
  (#1310)
   * Check for a new flox version on https://downloads.flox.dev, and if one is
     available, print a notification to the user.
   * Only print this notification once every 24 hours.
- Activate can now allow activation without prompt (#1144)
   * Adds a new config option `shell_prompt` with the possible values
     `show-all`, `hide-all` and `hide-default`.
   * `flox activate` respects this config and will **disable** the prompt
     modifications if the value is `hide-all`.
   * With `hide-default` it filters out all `default` environments, entirely
    disabling the prompt if all active envs were filtered.

### Go envs

- Now detect Go Projects during initialization. Thanks to Óscar Carrasco for
  the contribution. (#1227)

### Pull/Push Improvements

- `flox pull --force` now accepts `owner/name` as a parameter. Previously,
  force didn't allow for other parameters. (#1300)

### Packaging

- Flox RPMs and Debs now signed with a more compatible GPG key to allow for
  verification on a wider selection of Linux systems.
  (https://github.com/flox/flox-installers/pull/229 and
  https://github.com/flox/flox-installers/pull/232)
- Flox is now available as a homebrew cask. If you're a homebrew user, you can
  install via `brew install flox`.
  (https://github.com/Homebrew/homebrew-cask/pull/170971)


### Thank you to our community contributions this release

* [Minh Luu](https://github.com/PyGeek03) -  fix typo in README.md (#1221)
* [Óscar Carrasco](https://github.com/oxcabe)  - feat(commands): create Go init
  hook (#1227)
* [Óscar Carrasco](https://github.com/oxcabe) - refactor: reimplement
  `InitHook` trait to avoid invalid states caused by `should_run` (#1313)
