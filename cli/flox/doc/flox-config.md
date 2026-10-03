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
    Set to `false` to quiet both the reminder to log in and the warnings
    that resolution will soon require authentication.
    Authentication *errors* are unaffected: commands that require a
    login, such as `flox push`, still fail with an explanation.

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
    The path must be absolute.
    A relative path is an error.
    Flox uses the default when the value is empty.

`data_dir`
:   Directory where Flox should store persistent data
    (default: `$XDG_DATA_HOME/flox`).
    The path must be absolute.
    A relative path is an error.
    Flox uses the default when the value is empty.

`disable_hook`
:   Don't set up the Flox prompt hook as part of activation.
    The prompt hook is required for auto-activation and for `flox deactivate` to
    take effect (default: false).

`disable_metrics`
:   Set to `true` to turn off telemetry (default: `false`).
    Telemetry is the usage events, error reports and performance traces that Flox collects.
    Flox sends usage events to Flox.
    Flox sends error reports and performance traces to Sentry (`sentry.io`), a service that Flox uses.
    When telemetry is on, Flox stores a random device ID in `metrics-uuid` in the data directory, `~/.local/share/flox` by default.
    Usage events contain:

    * this device ID, a random ID for each run, and the `env_id` in `.flox/env.json`
    * your FloxHub account ID, when you are signed in or use a FloxHub token
    * whether you use a FloxHub login, a FloxHub token or neither
    * each command's name, time, exit code, duration and error category
    * the Flox version, your OS and kernel versions, your shell and CPU architecture
    * whether Flox runs in CI, VS Code or Imageless Kubernetes, and which AI coding tool runs it
    * the value of `FLOX_INVOCATION_SOURCE`, when it is set
    * the search terms you type, and the command names you pass to `flox search --command`
    * environment names, with the owner of each FloxHub environment
    * each environment's type, package count, generation and schema versions
    * for each activation: its mode, shell and invocation type, whether it starts services, and whether the environment includes other environments
    * whether `flox edit` changed the environment's includes, and whether you ran `flox generations list` with `--tree`
    * the packages you install, upgrade or uninstall, as written in your command or manifest, and whether each one succeeded
    * the old and new version of each package you upgrade
    * each build's type, outcome, duration, error category and lockfile hash
    * that you signed in, and that Flox showed you an update notice

    Unless you choose another name, a path environment's name is the name of the directory that holds its `.flox` directory, or `default` in your home directory.
    In an interactive or in-place activation, Flox's shell prompt hook records two usage events at every prompt, and on tcsh also at every directory change.
    Flox links the device ID to every FloxHub account that signs in on this device.
    Through that link, Flox also ties this device's signed-out events to that account.
    Flox links your FloxHub account ID to your account's email address.

    Error reports and performance traces carry the device ID and file paths.
    Error reports also carry the error message, and a stack trace when Flox crashes.
    Error reports also carry up to 100 recent Flox log messages, including messages that `-q` hides.
    Error reports from an activation also carry the output of the environment's `hook.on-deactivate` script.
    On macOS, error reports also carry your Mac model.
    Performance traces of commands that use FloxHub environments also carry the FloxHub URL.
    Performance traces also carry the search terms, package names and environment names of the command.
    Flox and Sentry receive your IP address when Flox sends telemetry.
    The Flox packages that Flox distributes (the installers, the Homebrew cask and the container images) send error reports and performance traces.
    Flox built from the `github:flox/flox` Nix flake does not send them.

    Telemetry goes to Flox and to Sentry even when `floxhub_url` points to a self-hosted FloxHub.
    For every field that telemetry contains, see [Flox data collection](https://flox.dev/docs/concepts/data-collection).
    For how long Flox keeps each kind of data, see [Retention](https://flox.dev/docs/concepts/data-collection#retention).

    When `disable_metrics` is `true`, Flox does not:

    * create `metrics-uuid` or print the first-run notice
    * record usage events or send them to Flox
    * look up your FloxHub account ID from FloxHub for telemetry
    * send error reports or performance traces
    * send the `flox-device-uuid`, `flox-invocation-id` and `sentry-trace` headers to FloxHub and the Flox Catalog

    Flox sets `FLOX_DISABLE_METRICS=true` for the processes it starts, so activated environments and their services inherit the setting.

    When telemetry is on, Flox creates the device ID on the first command that prints the notice.
    A command does not print the notice when `-q` or `RUST_LOG` hides Flox's messages, when it answers a shell completion request, or when Flox runs it in the background.
    Until a command prints the notice, every command runs as if `disable_metrics` were `true`, except that Flox does not set `FLOX_DISABLE_METRICS` for the processes it starts.

    Turning telemetry off does not stop the following:

    * Requests to FloxHub and the Flox Catalog that commands need.
      These requests contain:

      * the search terms and package names you type
      * the package groups, install IDs, packages, version constraints and systems in your manifest
      * the command name you pass to `flox run` or `flox search --command`
      * the store paths of packages from custom catalogs
      * the environments you push and pull
      * the packages you publish, with their git remote URL and revision

      `flox init` also sends the Node.js, Yarn, Python and Go version constraints that it reads from `package.json`, `.nvmrc`, `.node-version`, `pyproject.toml`, `go.work` and `go.mod`.
      When `.flox/catalog.lock` does not exist, `flox build`, `flox develop` and `flox publish` also send the names of the catalog packages that the Nix files in `.flox/pkgs` reference.
      `flox build update-catalogs` sends those names each time you run it.
      FloxHub and the Flox Catalog receive your IP address, the Flox version in the `user-agent` header, and your FloxHub token when you are signed in.
      Requests for FloxHub environments also carry your locale's language in the `Accept-Language` header.
      FloxHub records the search terms, command names, package names, package groups, version constraints and systems in catalog requests, with your IP address.
      When you are signed in, FloxHub also records your FloxHub account with them.
    * The `flox-invocation-source` header on those requests.
      It names CI, VS Code, Imageless Kubernetes or the AI coding tool that runs Flox, when Flox detects one.
      It also carries the value of `FLOX_INVOCATION_SOURCE`.
    * The background upgrade check that `flox activate` starts.
      It sends the packages in the environment's manifest to the Flox Catalog when the last successful check is more than 24 hours old, or when the lockfile changed.
      Flox 1.18.0 and later skip this request when you are signed out.
      For FloxHub environments, the check also fetches the environment from FloxHub.
      Setting `upgrade_notifications = false` hides the upgrade notice but does not stop the check.
    * Package downloads.
      Nix downloads packages from the binary caches in your Nix configuration, `cache.nixos.org` by default.
      Packages from custom catalogs download from the catalog's store.
      `flox build`, `flox develop` and `flox publish` download Nixpkgs from GitHub when Nix has not cached it.
      Nix downloads each flake reference that you install from the host that the reference names.
      Nix downloads the flake registry from `channels.nixos.org` when you install an indirect flake reference, for example `nixpkgs#hello`.
      Builds download the sources that they need.
      On macOS, `flox containerize` downloads the `nixos/nix` image from Docker Hub.
      It also downloads Flox from `github:flox/flox` and runs it in that image.
    * The update check.
      When `installer_channel` is set and Flox runs in an interactive terminal, Flox requests the latest version number from `downloads.flox.dev` when the last successful check is more than 24 hours old.
    * The generation history that Flox uploads to FloxHub.
      `flox push`, `flox init -r` and creating your FloxHub default environment upload this history with the manifest and lockfile of every generation.
      The history records your user name, your hostname, the full `flox` command line and the time of each change.
      Anyone who can pull the environment can read this history.
    * The random environment ID that `flox init` and `flox pull --copy` write to `.flox/env.json`.
      Flox sends this ID only as part of telemetry.
      When you commit `.flox/env.json` with your project, every clone of the repository has the same ID.
    * Activations that started while telemetry was on.
      They keep sending error reports and performance traces until they end.
    * The device ID and unsent events already on disk.
      Turning telemetry off does not delete `metrics-uuid` or the events that Flox recorded and has not sent.
      If you turn telemetry back on, Flox sends those events.
      On Flox 1.17.0, `flox config --set disable_metrics true` also sends those events, and its own, when the oldest is more than 2 minutes old.
    * The install script at `get.flox.dev`.
      The script sends one event when the installation starts and one when it ends.
      Both events contain the time, a random ID for the event, a random ID for the run, your OS, CPU architecture and release channel, and the device ID from `metrics-uuid` when that file exists.
      The first event also records the Flox version you set in `FLOX_VERSION`, and whether Nix and Flox are already installed.
      The second event records the Flox version, the outcome, the installation method and the reason for a failure.
      `disable_metrics` in a config file does not stop these events.
      To stop them, set `FLOX_DISABLE_METRICS=true` for the `sh` that runs the script: `curl -fsSL https://get.flox.dev | FLOX_DISABLE_METRICS=true sh`.

    Environment variables and your user config file override `/etc/flox.toml`.
    A `disable_metrics = true` in `/etc/flox.toml` is a default that each user can change.
    On macOS, installing or upgrading Flox with the `.pkg` installer or Homebrew replaces `/etc/flox.toml`.
    Set it again after each upgrade.

    When telemetry is on, `flox config --set disable_metrics true` runs with telemetry on, because Flox reads the configuration before the command changes it.
    In the packages that Flox distributes, that command sends a performance trace.
    If it is the first Flox command you run, it also creates `metrics-uuid` and prints the notice.
    If it is the first Flox command you run and `-q` hides the notice, it creates no `metrics-uuid` and sends no telemetry.
    To turn telemetry off before Flox sends anything, add `export FLOX_DISABLE_METRICS=true` to your shell profile, or add `disable_metrics = true` to a config file with a text editor, before you run Flox.
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
    The path must be absolute.
    A relative path is an error.
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
    See `disable_metrics` for what telemetry contains, what turning it off stops, and what it does not stop.
