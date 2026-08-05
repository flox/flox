# Flox Extensions — Author Guide

This guide is for people writing an extension. It covers source
layout, the `flox-extension.toml` manifest, and the local dev
loop.

## Contents

- [Source naming](#source-naming)
- [The `flox-extension.toml` manifest](#the-flox-extensiontoml-manifest)
- [Bookkeeping variables](#bookkeeping-variables)
- [Local dev loop](#local-dev-loop)
- [Example extensions](#example-extensions)

## Source naming

An extension is a directory whose name begins with `flox-`,
containing an executable of the same name. A directory named
`flox-tidy` installs as the `tidy` extension and is invoked as
`flox tidy`.

The executable can be written in any language — a shell or Python
script works as well as a compiled binary, as long as the file is
executable.

## The `flox-extension.toml` manifest

A `flox-extension.toml` at the source root is optional; it is
only needed if you want to name the extension explicitly instead
of deriving the name from the directory.

Minimal manifest:

```toml
schema = "1"

[extension]
name = "hello"
```

Full manifest with every field:

```toml
schema = "1"

[extension]
name = "deploy"
description = "Deploys things"
```

### `[extension]` table

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Extension name. Must match the source directory's `<name>` segment (directory `flox-<name>`) when both exist. Lowercased, `[a-z0-9][a-z0-9_-]*`. |
| `description` | string | no | Short human description. Not currently surfaced by `flox extension list` (which has no description column). |

### `schema` field

Always `"1"` for the current schema version.

## Bookkeeping variables

Flox injects three variables into the
extension's environment:

| Variable | Value |
|----------|-------|
| `FLOX_EXTENSION_NAME` | The extension's declared name. |
| `FLOX_EXTENSION_PATH` | Absolute path to the managed install directory, or to the executable itself when the extension was found on `$PATH` rather than installed. |
| `FLOX_BIN` | Path to the flox binary that dispatched the extension. |

Extensions can read these to find their own install directory
or to shell out to other `flox` subcommands.

## Local dev loop

During development, install your extension directly from a
local directory:

```console
$ cd ~/src/flox-hello
$ flox extension install --from-path .
✓ Installed flox-hello
$ flox hello
```

The installer derives the name from the directory basename
(stripping any leading `flox-`), or reads it from
`flox-extension.toml` if present.

To iterate, edit the source and re-install with `--force`:

```console
$ flox extension install --from-path . --force
```

The `--force` overrides the already-installed check. Alternately,
for pure script extensions, you can `flox extension remove` and
reinstall.

## Example extensions

- [**flox-hello-local**](https://github.com/flox/flox-hello-local)
  — canonical local-authoring reference. Clone the repo and
  install from the working tree via `flox extension install
  --from-path .`; demonstrates the [local dev loop](#local-dev-loop).

## See also

- [User guide](./user-guide.md)
- [Docs index](./README.md)
