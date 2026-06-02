---
title: FLOX-SERVICES-RESTART
section: 1
header: "Flox User Manuals"
...

# NAME

flox-services-restart - restart running services

# SYNOPSIS

```text
flox [<general-options>] services restart
     [-d=<path> | -r=<owner/name>]
     [<name>] ...
```

# DESCRIPTION

Restarts the specified services.

If no services are specified, stops all running services and starts new
services using the latest build of the environment. If one or more services
are running, then the specified services are started using the service config
that the running services were started with.

When all services are restarted, they are started from an ephemeral activation
that uses the latest build of the environment. This may not be the build of the
environment that your shell has activated, so the environment variables present
for services may be different from the ones in your shell. To ensure that your
shell and the services have the same environment, reactivate your environment
after making edits to the manifest.

An error is displayed if the specified service does not exist.

# OPTIONS

`<name>`
:   The name(s) of the services to restart.

```{.include}
./include/environment-options.md
./include/general-options.md
```

# EXAMPLES

Restart a single service:
```console
$ flox services restart myservice
✔ Service 'myservice' restarted.
```

Restart all services:
```console
$ flox services restart
✔ Service 'service1' restarted.
✔ Service 'service2' restarted.
✔ Service 'service3' restarted.
```

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md)

