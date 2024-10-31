---
title: FLOX-CONFIG
section: 1
header: "Flox User Manuals"
...


# NAME

flox-config - view and set configuration options

# SYNOPSIS

```
flox [<general-options>] config
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

Config values are read from the following sources in order of descending priority:

1. Environment variables.
   All config options may be set by prefixing with `FLOX_` and using
   SCREAMING_SNAKE_CASE.
   For example, `disable_metrics` may be set with `FLOX_DISABLE_METRICS=true`.
1. User customizations from `$FLOX_CONFIG_DIR/flox.toml` if set or else
   `$XDG_CONFIG_HOME/flox/flox.toml`.
1. User customizations from `flox/flox.toml` in any of `$XDG_CONFIG_DIRS`.
1. System settings from `/etc/flox.toml`.
1. `flox` provided defaults.

`flox config` commands that mutate configuration always write to
`${FLOX_CONFIG_DIR:-$XDG_CONFIG_HOME}/flox/flox.toml`.

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
:   Reset all options to their default values without confirmation.

`--set <key> <string>`
:  Set `<key> = <string>` for string values

`--set-number <key> <number>`
:  Set `<key> = <number>` for number values

`--set-bool <key> <bool>`
:  Set `<key> = <bool>` for boolean values

`--delete <key>`
:   Delete config key

```{.include}
./include/general-options.md
```

# SUPPORTED CONFIGURATION OPTIONS

`config_dir`
:   Directory where flox should load its configuration file
    (default: `$XDG_CONFIG_HOME/flox`).
    This option will only take effect if set with `$FLOX_CONFIG_DIR`.
    `$FLOX_CONFIG_DIR` and `config_dir` are ignored.

`cache_dir`
:   Directory where flox should store ephemeral data
    (default: `$XDG_CACHE_HOME/flox`).

`data_dir`
:   Directory where flox should store persistent data
    (default: `$XDG_DATA_HOME/flox`).

`disable_metrics`
:   Disable collecting and sending usage metrics.

`floxhub_token`
:   Token to authenticate on FloxHub.

`hide_default_prompt`
:   Hide environments named 'default' from the shell prompt,
    and don't add environments named 'default' to `$FLOX_PROMPT_ENVIRONMENTS` (default: true).

`search_limit`
:   How many items `flox search` should show by default.

`set_prompt`
:   Set shell prompt when activating an environment (default: true).

`shell_prompt` - DEPRECATED
:   Rule whether to change the shell prompt in activated environments
    (default: "show-all").
    This has been deprecated in favor of `set_prompt` and `hide_default_prompt`.
    Possible values are
    * "show-all": shows all active anvironments
    * "hide-all": disables the modification of the shell prompt
    * "hide-default": filters out environments named 'default' from the shell prompt

`trusted_environments`
:   Remote environments that are trusted for activation.
    Contains keys of the form `"<owner>/<name>"` that map to either `"trust"` or
    `"deny"`.

# ENVIRONMENT VARIABLES

`$FLOX_DISABLE_METRICS`
:   Variable for disabling the collection/sending of metrics data.
    If set to `true`, prevents Flox from submitting basic metrics information
    such as a unique token and the subcommand issued.
