---
title: FLOX-LIST
section: 1
header: "Flox User Manuals"
...


# NAME

flox-list - list packages installed in an environment

# SYNOPSIS

```text
flox [<general-options>] list
     [-d=<path> | -r=<owner/name>]
     [-u]
     [-e | -c | -n | -a]
```

# DESCRIPTION

List packages installed in an environment.
The options `-n`, `-e`, and `-a` exist to provide varying levels of detail in
the output.

With `-a`, the output also includes details about the environment itself,
such as its system, its generation, and the upgrades available for its
packages.
See [ALL DETAILS](#all-details) below.

# OPTIONS

## List Options

`-e`, `--extended`
:   Show the install ID, pkg-path, and version of each package (default).

`-c`, `--config`
:   Show the raw contents of the manifest.
    When using composition, the merged manifest will be shown without any
    commented lines.

`-n`, `--name`
:   Show only the install ID of each package.

`-a`, `--all`
:   Show all available information:
    details about the environment,
    followed by each package's priority, stability, license,
    outputs (both available and installed),
    and the upgrade available for it.
    A package's stability lists the stabilities of the catalog revision it
    is locked to, such as `stable, staging, unstable`.

```{.include}
./include/environment-options.md
./include/upstream-option.md
./include/general-options.md
```

# ALL DETAILS

With `-a`, the following details about the environment precede the packages.
Details that don't apply to an environment, or that can't be read,
are left out.

Environment
:   The name of the environment,
    including its owner for environments on FloxHub.

FloxHub URL
:   The page of the environment on FloxHub.

Path
:   The directory containing the environment.

System
:   The system the packages are listed for.

Systems
:   The systems the environment supports:
    `options.systems` from the manifest or, if that isn't set,
    the default systems its catalog packages are locked for.

Generation
:   For environments on FloxHub, the generation of the local copy:
    the live generation,
    or the generation that `flox activate --generation` pinned.
    Its packages are listed,
    unless `--upstream` lists the packages of FloxHub's live generation.
    It notes when a newer generation exists,
    for example after `flox generations rollback`,
    and when the environment has local changes
    that `flox edit --sync` hasn't committed to a generation yet.

FloxHub
:   The live generation on FloxHub,
    as of the last time the environment was fetched from FloxHub,
    for example by `flox pull`, `flox push`,
    or the check that `flox activate` runs in the background.
    With `--upstream`, the environment is fetched first.
    Compare it with the Generation line to see whether the local copy is
    ahead or behind.
    It notes when the local copy and FloxHub have diverged.

Auto-upgrade
:   For environments on FloxHub, whether FloxHub upgrades the environment
    automatically and how often, such as `weekly`,
    as of the last `flox list -a --upstream`
    or the check that `flox activate` runs in the background.
    `next due` is the next UTC date the schedule is due:
    FloxHub upgrades the environment when its scheduler runs on that date,
    and doesn't make up a run it missed.
    Once that date has passed, `next due` is left out
    until the settings are fetched again.
    It's `unknown` until the settings are fetched from FloxHub,
    if FloxHub didn't provide them,
    for example because you aren't logged in,
    or if the environment isn't on the configured FloxHub,
    the only one the settings are fetched from.

    Up to three more lines follow.
    `upgrades from` names the environment on FloxHub
    that the environment's packages are upgraded from,
    in place of upgrades from the Flox Catalog,
    if you can read that environment.
    The Upgrades line and `Upgrade available` still describe upgrades from
    the Flox Catalog.
    `last run` is when FloxHub last checked the environment for upgrades
    without upgrading it, if that came after its last upgrade,
    and whether no upgrades were available or the upgrade failed.
    `last upgraded` is when FloxHub last upgraded the environment on its own
    and the generation that upgrade created.

Upgrade notices
:   Whether `flox activate` notifies about available upgrades,
    as set by `upgrade_notifications` in
    [`flox-config(1)`](./flox-config.md)
    and by `options.activate.upgrade-notifications`
    in the manifest of the listed packages
    (see [`manifest.toml(5)`](./manifest.toml.md)).

Upgrades
:   A summary of the upgrades available for the listed packages
    on the current system.
    Each package with an upgrade available shows the version it upgrades to
    on an `Upgrade available` line, or `rebuild` for a new build of the same
    version.
    After the packages, the upgrades are listed together
    under `Available upgrades`, in the format of `flox upgrade --dry-run`.

Upgrades checked
:   When the upgrades were checked.
    The check runs in the background of `flox activate`
    when the environment has changed or hasn't been checked for a day,
    and only covers the live generation of the local copy.
    Run `flox upgrade --dry-run` to see the upgrades available now;
    it doesn't change what `flox list -a` shows.

The output of `-a` is meant to be read by people and its format may change.
Scripts should use `-n` or `-c` instead.

# SEE ALSO
[`flox-install(1)`](./flox-install.md)
[`flox-upgrade(1)`](./flox-upgrade.md)
[`flox-generations-list(1)`](./flox-generations-list.md)
