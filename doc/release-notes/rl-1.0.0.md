## Release 1.0.0 (2024-03-12)

This release addresses several bugs and adds new improvements, including:

 - `flox init` will detect existing project package managers and suggest packages and shell hook configuration. This works for some Python (pip, poetry) and Node (npm, yarn, nvm) projects. 
 - You can now set `FLOX_SHELL` to control the sub-shell Flox launches when using `flox activate`.
 - The Flox Catalog now contains `nodepackages` from Nixpkgs for searching and installation.
 - You can now search and install unfree packages in Flox. Flox will print a warning when installing a package that does not have open licensing.
 - Improved performance with better use of cache on subsequent environment activations.  
 - In situations where there are multiple environments active, Flox will ignore the default environment and not prompt for 'which environment' when a user is elsewhere on their machine. This makes it easier to use the default environment in your RC files. You can still update your default enviornment by running commands using the explicit `--dir` and `--remote` options or by being in the directory containing your default environment. 
 - Improved error message when an environment is not found during install and uninstall.
 - This release is compatible with https://auth.flox.dev and 0.9.0 users must upgrade to it to continue to `flox auth login` from the CLI.

We especially want to thank all our github.com/flox/flox contributors and
discourse community members for all your valuable feedback!

- Environments have improved script capabiltiies. Added a `[profile]` section to the manifest
- Renamed the `hooks.script` property to `hook.on-activate` and force the script to run in bash so it works consistently across users in different shells.
