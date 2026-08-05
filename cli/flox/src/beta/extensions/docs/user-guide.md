# Flox Extensions — User Guide

> **Beta:**
> Extensions are a beta feature and behind a feature flag, and their
> behavior is subject to change.
> Enable them with `flox config --set features.beta true` or by
> setting `FLOX_FEATURES_BETA=true` in your environment.

Flox extensions are out-of-tree commands that extend the `flox`
CLI. They are installed into
`$XDG_DATA_HOME/flox/extensions/` (typically
`~/.local/share/flox/extensions/`) and dispatched when you run
`flox <name>` where `<name>` is not a built-in subcommand.

Extensions can be written in any language. A `flox-hello`
extension is discovered and invoked as `flox hello`. An extension
runs as a plain child process and inherits the environment you
invoke it from.

## Contents

- [Quick tour](#quick-tour)
- [Installing extensions](#installing-extensions)
- [Listing and removing](#listing-and-removing)
- [Reserved names](#reserved-names)
- [Enabling extensions](#enabling-extensions)
- [See also](#see-also)

## Quick tour

```console
$ cd ~/src/flox-hello
$ flox extension install .
✔ Installed flox-hello -> ~/.local/share/flox/extensions/flox-hello

$ flox hello world
Hello from hello
args: world

$ flox extension list
NAME                  PATH
hello                 /home/me/src/flox-hello

$ flox extension remove hello
✔ Removed flox-hello
```

Note the name: the source directory is `flox-hello`, so the
extension is `hello` and is invoked as `flox hello`. The `flox-`
prefix is stripped; nothing else is.

## Installing extensions

### From a local path

```console
$ flox extension install .
$ flox extension install --from-path ./my-extension
```

Installs from a local directory — `.` for the current directory,
or `--from-path PATH` for an explicit one. The directory must
contain an executable named `flox-<name>`; `<name>` is derived
from the directory basename (stripping a leading `flox-`), or
read from `flox-extension.toml` if present. See the
[author guide](./author-guide.md) for the manifest schema and
local dev loop.

### Forcing a reinstall

```console
$ flox extension install . --force
```

`--force` overwrites an existing install at the same name. Use it
to pick up changes after editing the source — reinstalling is how
an extension updates.

## Listing and removing

### Listing installed extensions

```console
$ flox extension list
NAME                  PATH
hello                 /home/me/src/flox-hello
tidy                  /home/me/src/flox-tidy
```

The `PATH` column shows the source directory the extension was
installed from.

### Removing an extension

```console
$ flox extension remove <name>
```

This deletes the install directory. Any state kept inside the
install directory is removed with it.

## Reserved names

The extension installer rejects source directories whose derived
`<name>` collides with a built-in top-level `flox` subcommand. This
prevents an extension from shadowing a built-in if bpaf's parser
behavior ever changes.

Current reserved names:

- `init`, `envs`, `delete`
- `activate`, `deactivate`, `run`, `services`
- `search`, `show`
- `install`, `i`, `list`, `l`, `edit`, `include`, `upgrade`,
  `uninstall`, `generations`
- `build`, `publish`, `push`, `pull`, `containerize`
- `auth`, `config`, `gc`
- `extension`, `help`, `beta-enabled`, `factory`
- `reset-metrics`, `lock-manifest`, `check-for-upgrades`,
  `activation-state`, `services-socket`, `hook-env`

The last two groups are hidden commands that do not appear in
`flox --help`.

If you try to install `flox-install`, for example, the installer
returns a clear error and exits non-zero.

The authoritative list lives at
`cli/flox/src/beta/extensions/reserved.rs` in the
Flox repo.

## Enabling extensions

> **Beta:**
> Extensions are a beta feature and behind a feature flag, and their
> behavior is subject to change.

Extensions are **disabled by default**. Enable them persistently:

```console
$ flox config --set features.beta true
```

or per-shell by exporting the environment variable:

```console
$ export FLOX_FEATURES_BETA=true
```

Either route enables both the `flox extension …` subcommands and
the `flox <name>` dispatch fallback.

While the subsystem is in beta, `extension` does not appear in
`flox --help`.

## See also

- [Author guide](./author-guide.md) — `flox-extension.toml`
  schema and the local dev loop.
- [Docs index](./README.md)
