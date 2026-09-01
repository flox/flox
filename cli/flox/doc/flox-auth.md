---
title: FLOX-AUTH
section: 1
header: "Flox User Manuals"
...


# NAME

flox-auth - FloxHub authentication commands

# SYNOPSIS

```text
flox [<general-options>] auth
     (login [--token-file <path>] | logout | status | token)
```

# DESCRIPTION

Authenticate with FloxHub so that you can push and pull environments.

## Quieting login reminders

Flox reminds you to log in when you run a command without a FloxHub
account, and warns when it resolves packages against the catalog
without a login.
To turn these messages off:

```bash
flox config --set auth_notifications false
```

This suppresses advisory messages only.
Commands that require a login, such as `flox push`, still fail with
an explanation of what to do.

See [`flox-config(1)`](./flox-config.md).

# SUBCOMMANDS

## `login`

Logs in to FloxHub.

Required to interact with environments on FloxHub via `flox push`,
`flox pull`, and `flox activate -r`.
Authenticating also automatically trusts your personal environments.

Prompts you to enter a one-time code at a specified URL.
If called interactively it can open the browser for you if you press `<enter>`.

With `--token-file <path>` the login is non-interactive:
the FloxHub token is read from `<path>` instead
(pass `-` to read the token from stdin).
The file can contain a JWT access token, a personal access token,
or a service account token.
Token-file login does not open a browser or prompt for input.
A JWT access token is validated locally.
FloxHub must be reachable to validate personal access tokens and
service account tokens.
The validated token is stored.
Use this in CI, containers, and other scripted setups.

See also: [`flox-push(1)`](./flox-push.md),
[`flox-pull(1)`](./flox-pull.md),
[`flox-activate(1)`](./flox-activate.md)

## `logout`

Logs out from FloxHub.

## `status`

Print your current login status and token expiry when known.

## `token`

Print the current authentication token to stdout.
