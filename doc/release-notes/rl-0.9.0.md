## Release 0.9.0 (2024-02-15)

This release contains **breaking changes in the Flox CLI** and FloxHub service. You are responsible for recreating environments created in 0.3.6 and prior versions. If you rely on a command that has been removed, we recommend that you wait to upgrade until support is added. We'd love to hear from you on our discourse forum if you have questions.

 - Combined 'project environments' and 'managed environments' into a single 'environments' concept.
 - Flox now stores environment definition in a `.flox` folder in the current working directory by default.
 - Flox environments support multiple architectures without having to create multiple environments. 
 - The `--environment` or `-e` option for managed environments on many commands has been replaced with `-r` or `--remote` for FloxHub managed environments and `--dir` or `-d` to refer to local directory path environments.
 - Environments are now declared in a `manifest.toml` file located inside the `.flox` folder. `flox edit` will allow edits and validate changes made.
 - The FloxHub service and its associated commands are now available to anyone with a valid GitHub account. These commands will direct you to sign up: `flox push`, `flox pull`, `flox auth login`.
 - Several deprecated subcommands were removed. Notably the `build` and `publish` commands are no longer available in Flox CLI. We have plans to return the build and publish features to Flox in the future. Consult the `--help` or `man flox` for a full list of supported commands.
 - The Flox catalog that powers search and install is now tied to NixPkgs `23.11` stable branch. We have short term plans to expand the library of software in the Flox Catalog. `update` and `upgrade` will advance metadata index and software versions along the stable `23.11` branch.

We especially want to thank all our github.com/flox/flox contributors and
discourse community members for all your valuable feedback!
