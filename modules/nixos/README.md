# Flox NixOS module

NixOS modules provide a rich interface for
modeling configuration options for services,
setting required environment variables
and communicating various settings to related services.

The Flox NixOS module builds upon this functionality
to invoke services using Flox environments,
thus decoupling the O/S and application release cadence
and allowing faster iteration when developing and deploying services on NixOS.

There are two ways of configuring systemd services to run from Flox environments:

## Flox Services

This method configures systemd to activate environments with the
`flox activate --start-services` command,
delegating all process management thereafter
to the Flox services subsystem.

Example:
```nix
  services.flox = {
    enable = true;
    activations = {
      myechoip = {
        environment = "flox/echoip";
        trustEnvironment = true;
        floxHubTokenFile = "/run/keys/echoip.token";
        autoPull.enable = true;
      };
    };
  };
```

## Flox Overrides

This method leverages existing NixOS modules by providing the ability to
override the `ExecStart` option as required to run the service
from the activated Flox environment.

Example:
```nix
  services.flox.enable = true;
  systemd.services.echoip.flox = {
    environment = "flox/echoip";
    trustEnvironment = true;
    autoPull.enable = true;
    execStart = "echoip -l 127.0.0.1:8080 -H X-Real-IP";
  };
```

`services.flox.enable` switches on the units and state directory that both
methods share, so it is required for this method as well.
Without it nothing is added to the system and the override is not applied.

While the Services method presents the easiest/most intuitive interface
from a Flox perspective, the overrides approach makes it possible to leverage the
full capabilities of the NixOS module subsystem, as well as the hundreds
of existing NixOS modules maintained by the Nix community.

The overridden `ExecStart` runs the command inside `flox activate`,
so the process systemd tracks is the activation wrapper rather than
the daemon itself.
Units with `Type=simple` or `Type=exec` work as expected.
`Type=forking` is not supported (PID tracking would point at the
wrapper), and `Type=notify` units only become ready if systemd is
allowed to accept readiness notifications from child processes
(`NotifyAccess=all`).

Units using `DynamicUser` are not supported:
the environment is provisioned before the service starts,
so it must be owned by a static user.
For such units, force `DynamicUser` off and configure a static
user and group:
```nix
  users.users.echoip = { isSystemUser = true; group = "echoip"; };
  users.groups.echoip = { };
  systemd.services.echoip.serviceConfig = {
    DynamicUser = lib.mkForce false;
    User = "echoip";
    Group = "echoip";
  };
```

## How environments are provisioned and updated

Each service gets a working directory beneath `stateDir`
(default `/var/lib/flox`) holding the pulled environment.
A `flox-pull@<name>` unit provisions the environment on first start
and — unless `pullAtServiceStart` is disabled — refreshes it every time
the service starts.
A failed refresh of an already-provisioned environment does not prevent
the service from starting;
only the initial provisioning is a hard dependency.

With `autoPull.enable` a systemd timer pulls updates on the schedule
given by `autoPull.dates`.
With `autoRestart.enable` the service is additionally restarted whenever
a scheduled pull fetches a new generation of the environment;
without it, a pulled update takes effect the next time the service
restarts.

## Authentication

To pull private environments, point `floxHubTokenFile` at a file
containing a FloxHub token (readable by root).
The token is handed to the service as a systemd credential and reaches
`flox` only through the `FLOX_FLOXHUB_TOKEN` environment variable;
it never appears on a command line or in a configuration file.

## Common configuration attributes

The following configuration attributes are supported by both
of the Services and Overrides methods described above:

* `environment`
    The Flox environment to run the service from.
    Mandatory for the Services method;
    for the Overrides method a null value (the default)
    leaves the unit untouched.

    - _Type_: string
    - _Example_: "flox/default"

* `trustEnvironment`
    Whether to pass `--trust` when activating the environment.

    - _Type_: boolean
    - _Default_: `false`

* `floxHubTokenFile`
    Full path to a file containing a FloxHub token.

    - _Type_: null or path
    - _Default_: `null`
    - _Example_: "/run/secrets/floxhub/secret.token"

* `extraFloxArgs`
    Additional arguments to pass to every `flox` invocation.

    - _Type_: list of strings
    - _Default_: [ ]
    - _Example_: [ "-v" "-v" ]

* `extraFloxActivateArgs`
    Additional arguments to pass to `flox activate`.

    - _Type_: list of strings
    - _Default_: [ ]
    - _Example_: [ "--mode" "dev" ]

* `extraFloxPullArgs`
    Additional arguments to pass to `flox pull`.

    - _Type_: list of strings
    - _Default_: [ ]
    - _Example_: [ "-v" ]

* `pullAtServiceStart`
    Whether to refresh the Flox environment every time the service starts.
    The initial provisioning pull always happens regardless of this option.

    - _Type_: boolean
    - _Default_: `true`

* `autoPull.enable`
    Whether to pull the Flox environment on a schedule.

    - _Type_: boolean
    - _Default_: `false`

* `autoPull.dates`
    When and how often to pull updates,
    with format as described in `systemd.time(7)`.

    - _Type_: string
    - _Default_: `00:00`
    - _Example_: "daily"

* `autoRestart.enable`
    Whether to restart the service when a scheduled pull fetches a new
    generation of the environment.

    - _Type_: boolean
    - _Default_: `false`

## Flox Services configuration attributes

The following configuration attributes are supported by
the Services method only:

* `user`
    The user with which to run the service.
    When null, a `flox-<name>` system user is created for the service.
    `group` must be set when this option is set.

    - _Type_: null or string
    - _Default_: `null`

* `group`
    The primary group membership for the service invocation.

    - _Type_: null or string
    - _Default_: `null`

* `description`
    The systemd description for the service.

    - _Type_: null or string
    - _Default_: `null`
    - _Example_: "Foobar Web Server"

The Services method also supports the following module-wide options:

* `services.flox.stateDir`
    Path containing all state pertaining to Flox-managed services.

    - _Type_: path
    - _Default_: `/var/lib/flox`

* `services.flox.workingDirectoryMode`
    The mode of each service's working directory in numeric format.

    - _Type_: string
    - _Default_: `0700`

## Flox Overrides configuration attributes

The following configuration attributes are supported by
the Overrides method only:

* `execStart`
    The command to override the unit's ExecStart with.

    - _Type_: string
    - _Default_: `""`
    - _Example_: "echoip -l 127.0.0.1:8080 -H X-Real-IP"

* `script`
    Shell commands executed as the service’s main process,
    replacing the unit's own `script` if it has one.
    One of `execStart`, `script` or the unit's own `script`
    must be set.

    - _Type_: string
    - _Default_: `""`
    - _Example_:
        ```nix
          ''
            tmpdir=$(mktemp -d)
            trap "rm -rf $tmpdir" EXIT
            cd $tmpdir
            t3 -t output.log -- echoip -l 127.0.0.1:8080 -H X-Real-IP
          ''
        ```
