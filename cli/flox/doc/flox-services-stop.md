---
title: FLOX-SERVICES-STOP
section: 1
header: "Flox User Manuals"
...

# NAME

flox-services-stop - stop running services

# SYNOPSIS

```
flox [<general-options>] services stop
     [ -d=<path> ]
     [<name>] ...
```

# DESCRIPTION

Stops the specified running services.

If no services are specified, then all services will be stopped.
If any of the specified services are not currently running, a warning will be
displayed and the remaining services will be stopped.

If any of the specified services do not exist, an error will be returned
and no services will be stopped. If an error is encountered while stopping
one of the specified services, the remaining services will still be stopped
a warning will be displayed for the services that failed to stop, and a
non-zero exit code will be returned.


# OPTIONS

`-d`, `--dir`
:   Path containing a .flox/ directory.

`<name>`
:   The name(s) of the services to stop.

```{.include}
./include/general-options.md
```

# EXAMPLES:

Stop a running service named 'server':

```
$ flox services stop server
```

Stop all running services:

```
$ flox services stop
```

Attempt to stop a service that doesn't exist:
```
$ flox services stop myservice doesnt_exist
❌ ERROR: Service 'doesnt_exist' not found.  
```

Attempt to stop a service that isn't running:
```
$ flox services stop running not_running
⚠️  Service 'not_running' is not running
✅ Service 'running' stopped  
```

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md)
