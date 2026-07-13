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
The token is validated and stored,
and no browser, prompt, or network access is involved.
Use this in CI, containers, and other scripted setups.

See also: [`flox-push(1)`](./flox-push.md),
[`flox-pull(1)`](./flox-pull.md),
[`flox-activate(1)`](./flox-activate.md)

## `logout`

Logs out from FloxHub.

## `status`

Print your current login status.

## `token`

Print the current authentication token to stdout.
