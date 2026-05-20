### Testing [PR2873](https://github.com/flox/flox/pull/2873)

Be sure to use the `brantley/systemd-nixosModule` branch
when testing the Flox NixOS module:

#### Without flake

Update the remote for the clone found in `/etc/nixos/flox`,
pull the latest changes and rebuild:
```bash
git -C /etc/nixos/flox remote set-url origin 'https://github.com/flox/flox?ref=brantley/systemd-nixosModule'
git -C /etc/nixos/flox pull
nixos-rebuild switch
```

#### Without flake

Update the `flox` flake input and rebuild:
```bash
sed -i 's%"github:flox/flox"%"github:flox/flox/brantley/systemd-nixosModule"%' /etc/nixos/flake.nix
nixos-rebuild switch
```

This is temporary until the Flox NixOS module is merged into the main branch.
Once merged, you can revert to the use of `flox.url = "github:flox/flox"` instead.

# Flox NixOS module

NixOS modules provides a rich interface for
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
  systemd.services.echoip.flox = {
    environment = "flox/echoip";
    trustEnvironment = true;
    autoPull.enable = true;
    execStart = "echoip -l 127.0.0.1:8080 -H X-Real-IP";
  };
```

While the Services method presents the easiest/most intuitive interface
from a Flox perspective, the overrides approach makes it possible to leverage the
full capabilities of the NixOS module subsystem, as well as the hundreds
of existing NixOS modules maintained by the Nix community.

## Common configuration attributes

The following configuration attributes are supported by both
of the Services and Overrides methods described above:

* `environment` (mandatory)
    The Flox environment to use for the service.

    - _Type_: string
    - _Example_: "flox/default"

* `trustEnvironment`
    Whether to trust the environment using invocation option.

    - _Type_: boolean
    - _Default_: `false`

* `floxHubTokenFile`
    Full path to the FloxHub token file.

    - _Type_: null or path
    - _Default_: `null`
    - _Example_: "/run/secrets/floxhub/secret.token"

* `extraFloxArgs`
    Additional arguments to pass to `flox`.

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
    - _Example_: [ "--force" ]

* `pullAtServiceStart`
    Whether to pull the Flox environment at service start.

    - _Type_: boolean
    - _Default_: `false`

* `autoPull.enable`
    Whether to automatically pull the Flox environment.

    - _Type_: boolean
    - _Default_: `false`

* `autoPull.dates`
    How often or when upgrade occurs, with format as described in `systemd.time(7)`.

    - _Type_: string
    - _Default_: `00:00`
    - _Example_: "daily"

* `autoRestart.enable`
    Whether to automatically restart the service when the Flox environment changes.

    - _Type_: boolean
    - _Default_: `false`

* `stateDir`
    Path containing all state pertaining to Flox-managed services.

    - _Type_: path
    - _Default_: `/run`

## Flox Overrides configuration attributes

The following configuration attributes are supported by
the Overrides method only:

* `execStart`
    The command to override the unit's ExecStart with.

    - _Type_: null or string
    - _Default_: `null`
    - _Example_: "echoip -l 127.0.0.1:8080 -H X-Real-IP"

* `script`
    Shell commands executed as the service’s main process.

    - _Type_: null or string
    - _Default_: `null`
    - _Example_:
        ```nix
          ''
            tmpdir=$(mktemp -d)
            trap "rm -rf $tmpdir" EXIT
            cd $tmpdir
            t3 -t output.log -- echoip -l 127.0.0.1:8080 -H X-Real-IP
          ''
        ```
