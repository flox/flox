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
     [<name>] ...
```

# DESCRIPTION

Stops the specified running services.

If no services are specified, then all services will be stopped.

Attempting to stop a service that is not running will return an error.
Similarly, attempting to stop a service that isn't in your manifest will also
return an error.

# OPTIONS

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

# SEE ALSO
[`flox-activate(1)`](./flox-activate.md)
