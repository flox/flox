---
title: FLOX-AUTH
section: 1
header: "flox User Manuals"
...


# NAME

flox-auth - FloxHub authentication commands

# SYNOPSIS

```
flox [ <general-options> ] auth
     (login | logout)
```

# DESCRIPTION

Authenticate with FloxHub so that you can push and pull environments.

# OPTIONS

## login

Prompts you to enter a one-time code at a specified URL.
If called interactively it can open the browser for you if you press `<enter>`.
Uses GitHub for identity but otherwise does not acquire any permissions.

## logout

Logs out from FloxHub
