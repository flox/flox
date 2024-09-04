---
title: FLOX-SERVICES-LOGS
section: 1
header: "Flox User Manuals"
...

# NAME

flox-services-logs - show logs of services

# SYNOPSIS

```
flox [<general-options>] services logs
     [-d=<path> | -r=<owner/name>]
     [--follow]
     [-n=<num>]
     [<name>] ...
```

# DESCRIPTION

Display the logs of the specified services.

If no services are specified, then the `--follow` flag is required and logs
from all services will be printed in real time.

One or more service names specified with the `--follow` flag will follow the
logs for the specified services.

If a service name is supplied without the `--follow` flag then all of the
available logs are displayed for that service. If specified with the `-n` flag
then only the most recent `<num>` lines from that service are displayed.

An error will be returned if a specified service does not exist.

# OPTIONS

`-d`, `--dir`
:   Path containing a .flox/ directory.

`--follow`
:   Follow log output for the specified services. Required when no service
    names are supplied.

`-n`, `--tail`
:   Display only the last `<num>` lines from the logs of the specified
    services.

`<name>`
:   Which service(s) to display logs for. When omitted logs from all services
    will be displayed but the `--follow` flag is required.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# EXAMPLES:

Follow logs for all services:
```
$ flox services logs --follow
service1: hello
service2: hello
...
```

Follow logs for a subset of services:
```
$ flox services logs --follow service1 service3
service1: hello
service3: hello
...
```

Display all available logs for a single service:
```
$ flox services logs myservice
starting...
running...
stopping...
completed
```

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md)
[`flox-services-start(1)`](./flox-services-start.md)

