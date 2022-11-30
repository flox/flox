---
title: FLOX-CONFIG
section: 1
header: "flox User Manuals"
...


# NAME

flox-config - configure user parameters

# SYNOPSIS

flox [ `<general-options>` ] config [ (--list|-l) ] [ (--confirm|-c) ] [ (--reset|-r) ]
# DESCRIPTION

Configure and/or display user-specific parameters.



# OPTIONS

```{.include}
./include/general-options.md
./include/development-options.md
```

## Config Options

[ (--list|-l) ]
:   List the current values of all configurable parameters.

[ (--confirm|-c) ]
:   Prompt the user to confirm or update configurable parameters.

[ (--reset|-r) ]
：  Reset all configurable parameters to their default values without further confirmation.
