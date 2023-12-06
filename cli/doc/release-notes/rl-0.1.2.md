## Release 0.1.2 (2023-03-24)

This release introduces the ability to declare a project environment via a top-level `flox.nix` file.
This file is a Nix set (a map) which lists the packages to be included in the environment.
This way, you can `flox activate` at the project root, and instantiate an environment with all the packages you need to develop against that project.
See the [documentation](https://flox.dev/docs) for details.

- Added the ability to provide flox configuration files in TOML format, in 3 different places of the user's choosing read in the following order:
  - package defaults from `$PREFIX/etc/flox.toml` (PREFIX=${flox-bash})
  - installation defaults from `/etc/flox.toml`
  - user customizations from `$HOME/.config/flox/flox.toml`

- Added support for creating project-based flox environments with a `flox.nix` file in the project root along with supporting documentation.
This is in addition to being able to create project-based flox environments in the pkgs subdirectory as previously provided.

- Simplified default search behaviour and added verbose flag.
- Updated manpages and floxdocs to reflect above changes / enhancements.
- Improved error messages / UX output and fixed typos.

We especially want to thank all our `github.com/flox/flox` contributors!
