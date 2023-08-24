---
title: FLOX-CONFIG
section: 1
header: "flox User Manuals"
...


# NAME

flox-config - configure user parameters

# SYNOPSIS

flox [ `<general-options>` ] config ([(-l|\--list)] | (-r|\--reset) | --set `<key>` `<value>` | --set-number `<key>` `<number>` | --set-bool `<key>` `<bool>` | --delete `<key>`)

# DESCRIPTION

Configure and/or display user-specific parameters.

Without any flags or `-l` or `--list` shows all configurable options
with their computed value.

## Key Format

`<key>` supports dot-separated queries for nested vaules, e.g.

```
flox config --set 'nix.access_tokens."github.com"' "ghp_xxx"`
```

## Config Format

All config keys can be listed with

```
flox config
```

The dispalyed values are resolved by reading:

1. package defaults from `$PREFIX/etc/flox.toml`
2. installation defaults from `/etc/flox.toml`
3. user customizations from `$HOME/.config/flox/flox.toml`
4. environment variables

where the last occurence is used as the final value.

`flox config` commands that mutate configurations always write to `$HOME/.config/flox/flox.toml`.

# OPTIONS

```{.include}
./include/general-options.md
./include/development-options.md
```

## Config Options

[ (\--list|-l) ]
:   List the current values of all configurable parameters.

[ (\--reset|-r) ]
：  Reset all configurable parameters to their default values without further confirmation.

[ \--set `<key>` `<value>` ]
：  Set `<key> = <value>` for string values

[ \--set-number `<key>` `<value>`  ]
：  Set `<key> = <value>` for number values

[ \--set-bool `<key>` `<value>`  ]
：  Set `<key> = <value>` for boolean values

[ \--delete `<key>` ]
：  Reset the value for `<key>` to its default
