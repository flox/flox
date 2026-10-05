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

Without any flags or when `-l` is passed, `flox config` lists explicitly set values
and some built-in defaults.

Config values are read from the following sources in order of descending priority:

1. Environment variables.
   To set a config option with an environment variable, prefix its name with `FLOX_` and write it in `SCREAMING_SNAKE_CASE`.
   For example, `FLOX_DISABLE_METRICS=true` sets `disable_metrics` to `true`.
2. User customizations from `$FLOX_CONFIG_DIR/flox.toml` if set,
   otherwise `flox/flox.toml` in `$XDG_CONFIG_HOME` or any of `$XDG_CONFIG_DIRS`,
   wherever it is found first.
3. System settings from `/etc/flox.toml` or `FLOX_SYSTEM_CONFIG_DIR/flox.toml`.
4. `flox` provided defaults.

`DO_NOT_TRACK=true` sets `disable_metrics` to `true`, whatever the other sources set.

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
:   List explicitly set values and some built-in defaults.

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

`auth_notifications`
:   Print the advisory FloxHub authentication messages.
    Set to `false` to quiet the reminder to log in.
    Authentication *errors* are unaffected: commands that require a
    login, such as `flox push` or any command that resolves packages,
    still fail with an explanation.

    (default: true)

`auto_activate`
:   How auto-activation treats environments you have not yet allowed or denied.
    Possible values are `prompt` (default), `allowlist`, and `disabled`.
    `prompt` asks before auto-activating an environment the first time you enter
    its directory.
    `allowlist` skips the prompt and auto-activates only environments you have
    already allowed with `flox activate allow` or a prior prompt.
    `disabled` turns auto-activation off entirely: nothing is auto-activated
    even if previously allowed.
    See the *AUTO-ACTIVATION* section of [`flox-activate(1)`](./flox-activate.md).

`auto_activate_environments`
:   Per-directory auto-activation decisions.
    Keys are absolute paths to directories containing a `.flox` directory,
    each mapping to `allow` or `deny`.
    These are normally written for you by `flox activate allow` and
    `flox activate deny` rather than edited by hand.

    A key may also be a glob pattern, so that one entry covers many
    directories:
    `*` matches a single directory name and `**` matches any depth.
    An exact path always wins over patterns.
    If several patterns match a directory and disagree, the directory is
    denied.
    Patterns are matched against the canonical (symlink-resolved) directory
    path.
    Set patterns with `flox config --set`, quoting the key:

    ```bash
    flox config --set 'auto_activate_environments."/home/me/work/*"' allow
    flox config --set 'auto_activate_environments."/home/me/work/vendor/**"' deny
    ```

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
    Flox uses the default when the value is empty.

`data_dir`
:   Directory where Flox should store persistent data
    (default: `$XDG_DATA_HOME/flox`).
    Flox uses the default when the value is empty.

`disable_hook`
:   Don't set up the Flox prompt hook as part of activation.
    The prompt hook is required for auto-activation and for `flox deactivate` to
    take effect (default: false).

`disable_metrics`
:   Set to `true` to turn off telemetry (default: `false`).
    Telemetry is the usage events and error reports that Flox sends to Flox and to Sentry (`sentry.io`).
    Both carry a random device ID, which Flox links to your FloxHub account when you sign in.
    FloxHub logs the requests the CLI sends it, such as package searches and installs, with your IP address.
    Flox extensions can collect and send their own telemetry.
    For what Flox collects, see [Flox data collection](https://flox.dev/docs/concepts/data-collection).

    Turning telemetry off stops usage events and error reports.
    It does not stop the requests that features such as search and install need.
    It does not delete the device ID or the events that Flox saved but has not sent, and Flox sends them if you turn telemetry back on.

    `FLOX_DISABLE_METRICS=true` sets `disable_metrics` to `true`.
    `DO_NOT_TRACK=true` also does, and overrides `disable_metrics = false` from every source.
    To turn telemetry off before Flox sends anything, add `export FLOX_DISABLE_METRICS=true` or `export DO_NOT_TRACK=true` to your shell profile before you run Flox.
    Flox creates the device ID when it prints the first-run notice.
    To turn it off for every user on a machine, set `disable_metrics = true` in `/etc/flox.toml`.
    Environment variables and your user config file override `/etc/flox.toml`.
    On macOS, installing or upgrading Flox with the `.pkg` installer or Homebrew replaces `/etc/flox.toml`, so set it again after each upgrade.
    For CI, Imageless Kubernetes and NixOS, see [Turn off telemetry](https://flox.dev/docs/concepts/data-collection#turn-off-telemetry).

`floxhub_token`
:   Token to authenticate on FloxHub.

`hide_default_prompt`
:   Hide environments named 'default' from the shell prompt,
    and don't add environments named 'default' to `$FLOX_PROMPT_ENVIRONMENTS` (default: false).

`installer_channel`
:   Release channel to use when checking for updates to Flox.
    Valid values are `stable`, `nightly`, or `qa`.
    When `installer_channel` is not set, Flox does not check for updates.
    (default: not set)

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
    Flox uses the default when the value is empty.

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

    To turn off notifications for a single environment instead,
    set `options.activate.upgrade-notifications = false` in its manifest.
    See [`manifest.toml(5)`](./manifest.toml.md).

`keep_tempdir`
:   Flox creates a single tempdir for each process in
    `$FLOX_CACHE_HOME/process`.
    Flox will delete this tempdir upon conclusion of the process
    unless `keep_tempdir == true` AND verbose logs are enabled.

# ENVIRONMENT VARIABLES

`$FLOX_DISABLE_METRICS`
:   Set to `true` to turn off telemetry.
    This variable overrides `disable_metrics` in every config file.
    It accepts only `true` or `false`, in any letter case.
    Any other value, `1` included, makes Flox commands fail with a configuration error.
    `DO_NOT_TRACK=true` overrides `FLOX_DISABLE_METRICS=false`.
    See `disable_metrics` for what telemetry is and what turning it off does not stop.

`$DO_NOT_TRACK`
:   Set to `true` to turn off telemetry.
    Flox treats every value except an empty value, `0` and `false` as `true`.
    Flox ignores letter case and surrounding whitespace in the value.
    `DO_NOT_TRACK` turns off everything that `disable_metrics = true` turns off.
    It overrides `FLOX_DISABLE_METRICS=false` and `disable_metrics = false` in every config file.
    The install script at `get.flox.dev` does not read `DO_NOT_TRACK`.
    See `disable_metrics` for what telemetry is and what turning it off does not stop.
