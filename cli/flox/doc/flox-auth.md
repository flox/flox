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

## Linux keyrings

New credentials are stored in the default keyring selected in your keyring app.
Existing credentials are updated where they were found, with priority given to
matching credentials in a keyring named `Login`.
An empty `Login` keyring does not override your default.
Logout removes matching credentials from all keyrings for the current FloxHub.

When the selected keyring is locked, GNOME Keyring can be unlocked with hidden
password entry in the terminal.
SSH sessions and sessions without an X11 or Wayland display use terminal
unlocking directly.
Desktop sessions first open the graphical unlock prompt.
Press Enter in the terminal to switch immediately to terminal unlocking if the
window is inaccessible.
A failed or dismissed desktop prompt, or a 30-second timeout, also falls back to
terminal entry.
Press Ctrl-C in the terminal to cancel the unlock operation.
Other keyring providers must be unlocked through their desktop app.
Press Esc to cancel terminal password entry; incorrect passwords can be retried.
The keyring password is never saved by Flox.

Pipelines, prompt hooks, and `login --token-file` do not open unlock prompts.
Unlock the keyring before running a command that needs its credentials.
Cancelling or failing to unlock an available keyring does not save the new
credential in plain text.

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
