## Release 0.3.1 (2023-09-02)

This release includes improvements and addresses critical bugs encountered with the recent 0.3.0 release, including:
- Replaced references to "floxdev.com" with our new domain "flox.dev".
- Fixed bug that had been introducing double "system" components in environment links and corrupting the history of
  version upgrades as reported with `flox history`.
- Fixed bug affecting ability to roll back to old generations not present on disk.
- Fixed bug patching `/etc/zshrc{,_Apple_Terminal}` files when running `flox activate` from a shell "rc" file.
- Fixed bug introduced with version 0.3.0 affecting the creation of managed environments.
- Rebuilds flox against the latest stable nixpkgs to avoid [stream reset errors](https://github.com/curl/curl/issues/11353)
  addressed by `curl` version 8.2.1.

We especially want to thank all our github.com/flox/flox contributors and discourse community members for all your valuable feedback!
