---
title: FLOX-CONFIG
section: 1
header: "Flox User Manuals"
...


# NAME

flox-config - view and set configuration options

# SYNOPSIS

```text
flox [<general-options>] config
     [-l |
      -r |
      --set <key> <string> |
      --delete=<key>]
```

# DESCRIPTION

Without any flags or when `-l` is passed, `flox config` shows all options with
their computed value.

Config values are read from the following sources in order of descending priority:

1. Environment variables.
   All config options may be set by prefixing with `FLOX_` and using
   `SCREAMING_SNAKE_CASE`.
   For example, `disable_metrics` may be set with `FLOX_DISABLE_METRICS=true`.
2. User customizations from `$FLOX_CONFIG_DIR/flox.toml` if set,
   otherwise `flox/flox.toml` in `$XDG_CONFIG_HOME` or any of `$XDG_CONFIG_DIRS`,
   wherever it is found first.
3. System settings from `/etc/flox.toml` or `FLOX_SYSTEM_CONFIG_DIR/flox.toml`.
4. `flox` provided defaults.

`flox config` commands that mutate configuration always write to the user config file
determined in step 2.


## Key Format

`<key>` supports dot-separated queries for nested values, for example:

```bash
flox config --set 'trusted_environments."owner/name"' trust
```

# OPTIONS

## Config Options

`-l`, `--list`
:   List the current values of all options.

`-r`, `--reset`
:   Reset all options to their default values without confirmation.

`--set <key> <string>`
:  Set `<key> = <string>` for a config key

`--delete <key>`
:   Delete config key

```{.include}
./include/general-options.md
```

# SUPPORTED CONFIGURATION OPTIONS

`auto_activate`
:   How auto-activation treats environments you have not yet allowed or denied,
    when the `auto_activate` feature flag is enabled.
    Possible values are `prompt` (default) and `allowed`.
    `prompt` asks before auto-activating an environment the first time you enter
    its directory.
    `allowed` skips the prompt and auto-activates only environments you have
    already allowed with `flox activate allow`.
    See the *AUTO-ACTIVATION* section of [`flox-activate(1)`](./flox-activate.md).

`auto_activate_environments`
:   Per-directory auto-activation decisions.
    Keys are absolute paths to directories containing a `.flox` directory,
    each mapping to `allow` or `deny`.
    These are normally written for you by `flox activate allow` and
    `flox activate deny` rather than edited by hand.

`auto_activate_fish_mode`
:   Controls how the `fish` shell hook responds to directory changes during
    auto-activation, mirroring direnv's `direnv_fish_mode`.
    Possible values are `eval_on_arrow` (default), `eval_after_arrow`, and
    `disable_arrow`.
    `eval_on_arrow` evaluates on prompt and immediately when the working
    directory changes.
    `eval_after_arrow` evaluates on prompt and defers directory-change
    evaluation until just before the next command runs.
    `disable_arrow` evaluates on prompt only, ignoring directory changes.

`config_dir`
:   Directory where Flox should load its configuration file
    (default: `$XDG_CONFIG_HOME/flox`).
    This option will only take effect if set with `$FLOX_CONFIG_DIR`.
    `config_dir` is ignored.

`cache_dir`
:   Directory where Flox should store ephemeral data
    (default: `$XDG_CACHE_HOME/flox`).

`data_dir`
:   Directory where Flox should store persistent data
    (default: `$XDG_DATA_HOME/flox`).

`disable_hook`
:   Don't set up the Flox prompt hook as part of activation.
    The prompt hook is required for auto-activation and for `flox deactivate` to
    take effect (default: false).

`disable_metrics`
:   Disable collecting and sending usage metrics.

`features.auto_activate`
:   Feature flag to enable auto-activation, which is experimental
    (default: false).
    May also be set with `FLOX_FEATURES_AUTO_ACTIVATE=true`.
    See the *AUTO-ACTIVATION* section of [`flox-activate(1)`](./flox-activate.md).

`floxhub_token`
:   Token to authenticate on FloxHub.

`hide_default_prompt`
:   Hide environments named 'default' from the shell prompt,
    and don't add environments named 'default' to `$FLOX_PROMPT_ENVIRONMENTS` (default: true).

`installer_channel`
:   Release channel to use when checking for updates to Flox.
    Valid values are `stable`, `nightly`, or `qa`.
    (default: `stable`)

`search_limit`
:   How many items `flox search` should show by default.

`set_prompt`
:   Set shell prompt when activating an environment (default: true).

`shell_prompt` - DEPRECATED
:   Rule whether to change the shell prompt in activated environments
    (default: `show-all`).
    This has been deprecated in favor of `set_prompt` and `hide_default_prompt`.
    Possible values are:

    * `show-all`: shows all active environments
    * `hide-all`: disables the modification of the shell prompt
    * `hide-default`: filters out environments named `default` from the shell prompt

`state_dir`
:   Directory where Flox should store data that's not critical but also
    shouldn't be able to be freely deleted like data in the cache directory.
    (default: `$XDG_STATE_HOME/flox` e.g. `~/.local/state/flox`)

`trusted_environments`
:   Remote environments that are trusted for activation.
    Keys are of the form `"<owner>/<name>"` or can include a wildcard for environment names `"<owner>/*"`.
    Values can be `"trust"` or `"deny"`

`upgrade_notifications`
:   Print notification if upgrades are available on `flox activate`.
    The notification message is:
    ```console
    Upgrades are available for packages in 'environment-name'.
    Use 'flox upgrade --dry-run' for details.
    ```

    (default: true)

`keep_tempdir`
:   Flox creates a single tempdir for each process in   
    `$FLOX_CACHE_HOME/process`.   
    Flox will delete this tempdir upon conclusion of the process
    unless `keep_tempdir == true` AND verbose logs are enabled.

# ENVIRONMENT VARIABLES

`$FLOX_DISABLE_METRICS`
:   Variable for disabling the collection/sending of metrics data.
    If set to `true`, prevents Flox from submitting basic metrics information
    such as a unique token and the subcommand issued.

`$FLOX_FEATURES_AUTO_ACTIVATE`
:   Set to `true` to enable auto-activation, which is experimental.
    Equivalent to setting `features.auto_activate = true`.
    See the *AUTO-ACTIVATION* section of [`flox-activate(1)`](./flox-activate.md).
