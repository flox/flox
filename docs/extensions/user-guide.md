# Flox Extensions — User Guide

> **Beta:**
> Extensions are a beta feature and behind a feature flag, and their
> behavior is subject to change.
> Enable them by setting `FLOX_FEATURES_BETA=true` in your environment.
> See [Enabling extensions](#enabling-extensions) for why the
> `flox config` route alone is not enough for `flox <name>` dispatch.

Flox extensions are out-of-tree commands that extend the `flox`
CLI. They are installed into
`$XDG_DATA_HOME/flox/extensions/` (typically
`~/.local/share/flox/extensions/`) and dispatched when you run
`flox <name>` where `<name>` is not a built-in subcommand.

Extensions can be written in any language. A `flox-hello`
extension is discovered and invoked as `flox hello`. Extensions
integrate with Flox environments through an optional manifest
that specifies activation behavior.

## Contents

- [Quick tour](#quick-tour)
- [Installing extensions](#installing-extensions)
- [Listing, removing, upgrading](#listing-removing-upgrading)
- [Activation modes](#activation-modes)
- [Reserved names](#reserved-names)
- [GitHub Enterprise](#github-enterprise)
- [Enabling extensions](#enabling-extensions)
- [See also](#see-also)

## Quick tour

```console
$ flox extension install flox/flox-hello-script
✔ Installed flox-hello-script

$ flox hello-script world
Hello from hello-script vc8d5f41
args: world

$ flox extension list
NAME                  REPO                              VERSION         PINNED  STATUS
hello-script          flox/flox-hello-script            c8d5f41

$ flox extension remove hello-script
✔ Removed flox-hello-script
```

Note the name: the repo is `flox-hello-script`, so the extension
is `hello-script` and is invoked as `flox hello-script`. The
`flox-` prefix is stripped; nothing else is.

## Installing extensions

### From GitHub

```console
$ flox extension install <owner>/<repo>
```

The spec must be exactly `<owner>/<repo>` (full URLs are not
accepted), and the repo must already begin with `flox-`. So
`acme/flox-hello` installs as the `hello` extension; `acme/hello`
is rejected — there is no auto-prefixing.

The installer picks a source strategy based on the repo:

1. If the latest GitHub release has a matching release asset
   (see the [author guide](./author-guide.md) for asset naming),
   Flox downloads and extracts the binary directly.
2. Otherwise, Flox `git clone`s the repo into the install
   directory and runs the executable from there (script-kind).

### Pinning a specific commit or tag

```console
$ flox extension install --pin v1.2.3 <owner>/<repo>
$ flox extension install --pin abc1234 <owner>/<repo>
```

A pinned extension will not upgrade past the pinned revision
without `--force`. This is the right choice when you want a
reproducible install, or when newer versions of the extension
have broken behavior you depend on.

### Forcing a reinstall

```console
$ flox extension install --force <owner>/<repo>
```

`--force` overwrites an existing install at the same name. Use
this when you want to reset an extension's state or when you're
overriding a pin.

### Installing from a local path

```console
$ flox extension install --from-path ./my-extension
```

Installs directly from a local directory, skipping GitHub
entirely. The directory must contain an executable named
`flox-<name>`; `<name>` is derived from the directory basename
(stripping a leading `flox-`), or read from
`flox-extension.toml` if present. See the
[author guide](./author-guide.md) for the manifest schema and
local dev loop.

## Listing, removing, upgrading

### Listing installed extensions

```console
$ flox extension list
NAME                  REPO                              VERSION         PINNED  STATUS
hello-script          flox/flox-hello-script            c8d5f41
tidy                  acme/flox-tidy                    v1.2.3          yes
```

The `VERSION` column shows the pinned tag when one exists,
otherwise the short commit SHA, otherwise blank. `PINNED` is
`yes` for extensions installed with `--pin`. `STATUS` is
populated by `upgrade`; it is blank for `list`.

### Removing an extension

```console
$ flox extension remove <name>
```

This deletes the install directory. Any state kept inside the
install directory is removed with it.

### Upgrading a single extension

```console
$ flox extension upgrade <name>
```

Resolves the latest commit (or release) for the extension's
tracked ref and replaces the install in place. If there's
nothing newer, Flox reports that the extension is already
current and exits successfully. If the extension is pinned, the
upgrade is a no-op and Flox tells you to pass `--force` to
override.

### Upgrading everything at once

```console
$ flox extension upgrade --all
```

Iterates every installed extension and prints a row per result:

```
NAME                  REPO                              VERSION         PINNED  STATUS
hello-script          flox/flox-hello-script            c8d5f41                 up-to-date
tidy                  acme/flox-tidy                    def5678                 upgraded abc1234 -> def5678
report                acme/flox-report                  v1.2.3          yes     pinned (skip)
```

Pinned extensions are skipped unless `--force` is passed.

### Dry-run

```console
$ flox extension upgrade --all --dry-run
```

Shows what would happen without mutating anything. Each row
reports `would upgrade <old> -> <new>`, `up-to-date`, or
`pinned (skip)`.

## Activation modes

An extension can declare how it interacts with Flox environments
by setting `[environment].mode` in its
`flox-extension.toml`. Flox honors one of three modes when
dispatching `flox <name>`:

### `inherit` (default)

The extension runs inside the caller's current activation. All
`FLOX_*` environment variables are preserved; if the caller is
already inside `flox activate`, the extension sees that
activation.

```toml
[environment]
mode = "inherit"
```

This is the right default for most extensions — they behave
like any other command you'd run at the prompt.

### `none`

The extension runs with a scrubbed environment. Before exec,
Flox calls `env_clear()` and replays every non-`FLOX_*` variable,
then overlays the extension-bookkeeping variables
(`FLOX_EXTENSION_NAME`, `FLOX_EXTENSION_VERSION`,
`FLOX_EXTENSION_PATH`, `FLOX_BIN`).

```toml
[environment]
mode = "none"
```

Use `none` when the extension must not be influenced by
caller-side Flox state. Examples: a diagnostic tool that reports
Flox state itself, or a sandboxed auditor.

### `pinned`

The extension runs inside a specific activation. When the
caller is already activated in the pinned environment, Flox
execs the extension directly. Otherwise, Flox wraps the call
with `flox activate -r <owner>/<env> -- <extension>`.

```toml
[environment]
mode = "pinned"
inherit_name = "acme/dev"
```

`inherit_name` takes the `<owner>/<environment>` reference that
would be valid for `flox activate -r`.

Optionally, an extension can refuse to run from outside its
pinned environment when the user is already activated in a
different one:

```toml
[environment]
mode = "pinned"
inherit_name = "acme/dev"

[on_active]
inside = "error"
```

`inside = "error"` causes the dispatch to fail with a
`PinnedEnvMismatch` error rather than silently re-wrapping. The
default (`inside = "override"`) just wraps with
`flox activate -r` unconditionally.

## Reserved names

The extension installer rejects repos whose `<name>` segment
collides with a built-in top-level `flox` subcommand. This
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

The last two (`beta-enabled`, `factory`) plus `extension` and
`help` are hidden commands that do not appear in `flox --help`.

If you try to install `flox-install`, for example, the installer
returns a clear error and exits non-zero.

The authoritative list lives at
`cli/beta/src/extensions/reserved.rs` in the
Flox repo.

## GitHub Enterprise

GitHub Enterprise is **not currently supported**. Flox talks to
`api.github.com` for metadata and clones from `https://github.com`;
the clone host is hardcoded and there is no configuration to point
it elsewhere.

There is one override, `FLOX_EXTENSIONS_GITHUB_BASE_URL`, but it is
**test-only** — it redirects the API client at a mock server for
the integration tests and does not change the `git clone` host, so
it cannot be used to install from a GHE instance.

## Enabling extensions

> **Beta:**
> Extensions are a beta feature and behind a feature flag, and their
> behavior is subject to change.

Extensions are **disabled by default**. Enable them by exporting the
environment variable:

```console
$ export FLOX_FEATURES_BETA=true
```

While the subsystem is in beta, `extension` does not appear in
`flox --help`.

### Enabling via config is not sufficient for dispatch

Beta features can also be enabled persistently:

```console
$ flox config --set features.beta true
```

This enables the `flox extension …` subcommands, but **not** the
`flox <name>` dispatch fallback.

Dispatch is resolved in `main()` before the config system has
loaded, so it reads `FLOX_FEATURES_BETA` from the environment
directly and never sees a config-file setting. If you enable beta
via config and want `flox hello` to work as well as
`flox extension install`, export the variable too:

```console
$ flox config --set features.beta true   # subcommands
$ export FLOX_FEATURES_BETA=true         # subcommands + dispatch
```

Add the export to your shell profile to make it permanent. This
limitation is expected to be resolved before extensions leave beta.

The variable is read as opt-in: `true` and `1` enable the feature
(case-insensitive). Any other value, or leaving it unset, keeps it
disabled.

## See also

- [Author guide](./author-guide.md) — repo layout,
  `flox-extension.toml` schema, release assets, local dev loop.
- [Docs index](./README.md)
