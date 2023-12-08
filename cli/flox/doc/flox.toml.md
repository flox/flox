---
title: FLOX-TOML
section: 1
header: "flox User Manuals"
...

flox configuration files in TOML format - BETA

# Description

flox provides the user some ability to control the flox configuration via files in TOML format.
There are 3 configuration files available which are read in the following order:

1. package defaults from `$PREFIX/etc/flox.toml` (PREFIX=${flox})
2. installation defaults from `/etc/flox.toml`
3. user customizations from `$HOME/.config/flox/flox.toml`

# FIELDS

- `disable_metrics = false` 
  - corresponds to `$FLOX_DISABLE_METRICS=(true|false)`
  - allows disabling metrics
  - semantics may change 
- `cache_dir = "/Users/floxfan/.cache/flox/"`
  - directory for flox' cache data
- `data_dir = "/Users/floxfan/.local/share/flox/"`
  - directory for flox' persistent data (rendered environments)
- `config_dir = "/Users/floxfan/.config/flox/" `
  - not configurable, included for completenes
- `stability = "stable"`
  - default stability of the flox instance
  - corresponds to `--stability <stability>` and `$FLOX_STABILITY=<stability>`
  - priority order: flag, env, config file
- `default_substituter = "https://cache.floxdev.com/"`
  - default cache to look up artifacts from
- `git_base_url = "https://github.com/"`
  - assumes github(-enterprise) or github-like git forges
  - changed from `floxpkgs.gitBaseURL`
- `nix = { access_tokens = {} }`
  - some nix configuration options
  - currently limited to access tokens
  - may allow arbitrary nix config values later
- `features = {}`
  - feature flags
  - current supported flags are
    - `all = ("bash"|"rust")` use **`"bash"`** or `"rust"` impl for all commands
    - `nix = ("bash"|"rust")` use `"bash"` or **`"rust"`**  for nix passthru commands
    - `env = ("bash"|"rust")` use **`"bash"`** or `"rust"`  for environment commands
    - `develop = ("bash"|"rust")` use **`"bash"`** or `"rust"`  for `develop` command
    - `publish = ("bash"|"rust")` use **`"bash"`** or `"rust"`  for `publish` command

## SEE ALSO

[`flox-config`(1)](./flox-config.md),
