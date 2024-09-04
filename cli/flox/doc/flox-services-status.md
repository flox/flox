---
title: FLOX-SERVICES-STATUS
section: 1
header: "Flox User Manuals"
...

# NAME

flox-services-status - display the status of services

# SYNOPSIS

```
flox [<general-options>] services status
     [-d=<path> | -r=<owner/name>]
     [--json]
     [<name>] ...
```

# DESCRIPTION

Displays the status of one or more services.

If no services are specified, then all services will be displayed. If no
services have been started for this environment, an error will be displayed.
An error will also be displayed if one of the specified services
does not exist.

# OPTIONS

`-d`, `--dir`
:   Path containing a .flox/ directory.

`--json`
:   Print statuses formatted as JSON. Each service is printed as a single JSON
    object on its own line.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# EXAMPLES:

Display statuses for all services:
```
$ flox services status
NAME       STATUS            PID
sleeping   Running         89718
myservice  Running         12345
```

Display the status of a single service:
```
$ flox services status myservice
NAME       STATUS            PID
myservice  Running         12345
```

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md)
[`flox-services-start(1)`](./flox-services-start.md)
