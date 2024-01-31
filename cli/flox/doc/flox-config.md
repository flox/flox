---
title: FLOX-CONFIG
section: 1
header: "flox User Manuals"
...


# NAME

flox-config - view and set configuration options

# SYNOPSIS

```
flox [ <general-options> ] config
     [-l |
      -r |
      --set <key> <string> |
      --set-number <key> <number> |
      --set-bool <key> <bool> |
      --delete=<key>]
```

# DESCRIPTION

Without any flags or when `-l` is passed, `flox config` shows all options with
their computed value.

The values are computed by reading:

1. `flox` provided defaults.
2. System settings from `/etc/flox.toml`.
3. User customizations from `$XDG_CONFIG_DIRS`.
4. User customizations from `$FLOX_CONFIG_HOME` if set or else `$XDG_CONFIG_HOME`.
4. Environment variables.
   All config options may be set by prefixing with `FLOX_` and using
   SCREAMING_SNAKE_CASE.
   For example, `disable_metrics` may be set with `FLOX_DISABLE_METRICS=true`.

The last occurence is used as the final value.

`flox config` commands that mutate configuration always write to
`${FLOX_CONFIG_HOME:-$XDG_CONFIG_HOME}/flox/flox.toml`.

## Key Format

`<key>` supports dot-separated queries for nested values, for example:

```
flox config --set 'trusted_environments."owner/name"' trust
```

# OPTIONS

## Config Options

`-l`, `--list`
:   List the current values of all options.

`-r`, `--reset`
:   Reset all options to their default values without further confirmation.

`--set <key> <value>`
:  Set `<key> = <value>` for string values

`--set-number <key> <value>`
:  Set `<key> = <value>` for number values

`--set-bool <key> <value>`
:  Set `<key> = <value>` for boolean values

`--delete <key>`
:   Delete config key

```{.include}
./include/general-options.md
```

# SUPPORTED CONFIGURATION OPTIONS

`config_dir`
:   Directory where flox should load its configuration file (default:
    `$XDG_CONFIG_HOME/flox`).
    This option will only take effect if set with `$FLOX_CONFIG_HOME`.
    `$FLOX_CONFIG_DIR` and `config_dir` are ignored.

`cache_dir`
:   Directory where flox should store ephemeral data (default:
    `$XDG_CACHE_HOME/flox`).

`data_dir`
:   Directory where flox should store persistent data (default:
    `$XDG_DATA_HOME/flox`).

`disable_metrics`
:   Disable collecting and sending usage metrics.

`floxhub_token`
:   Token to authenticate on FloxHub.

`search_limit`
:   How many items `flox search` should show by default.

`trusted_environments`
:   Remote environments that are trusted for activation.
    Contains keys of the form `<owner/name>` that map to either `trust` or
    `deny`.

## Options used internally

`nix`
:   Options to pass to `nix` commands.

`floxhub_url`
:   The URL of the FloxHub instance to use.

`features`
:   Options used for feature flagging.
