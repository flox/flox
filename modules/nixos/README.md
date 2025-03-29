# Flox NixOS module

The Flox NixOS module provides
a convenient way of installing Flox
while also making it easy to run `systemd`-managed services
from Flox environments.
This makes it possible to decouple
the O/S and application release cadence,
making it faster to iteratively develop and deploy services on NixOS.

## Installation

### Use flakes for your NixOS installation

_**Note**_: the use of the Flox NixOS module currently requires that
you manage your NixOS configuration as a flake in `/etc/nixos/flake.nix`.
This aspect of the configuration is outside of the scope of this README
but you can find an excellent introduction in
[Ryan Yin's](https://github.com/ryan4yin)
[NixOS with flakes enabled](https://nixos-and-flakes.thiscute.world/nixos-with-flakes/nixos-with-flakes-enabled)
tutorial on the topic.

### Install Flox NixOS module

Once your NixOS configuration is configured with flakes,
installing the Flox NixOS module requires just a few simple steps:

1. add `flox.url = "github:flox/flox/brantley/systemd-nixosModule"` to `inputs`
    * This is a temporary URL until the Flox NixOS module is merged into the main branch.
      Once merged, you can use `flox.url = "github:flox/flox"` instead.
1. within the `outputs.nixosConfigurations.<hostname>` attribute set:
    1. apply the `rec` (recursive) keyword to the attribute set
    1. set `specialArgs = { inherit system; }`
    1. add `inputs.flox.nixosModules.flox` to the `modules` array
1. [_**OPTIONAL**_] set `nixpkgs.url = "github:flox/nixpkgs/stable"` in `inputs`
    * The `github:flox/nixpkgs/stable` fork of nixpkgs
      tracks the `github:/nixos/nixpkgs/nixos-unstable` branch
      through a series of cascading branches curated by Flox.
      Building your system with this Nixpkgs URL
      will provide the optimal likelihood of
      closure overlap with Flox environments
      and minimize the size of the Nix store,
      but is _**not**_ required for the use of Flox on NixOS.

The resulting Nix flake should look something like the following:
```nix
{
  inputs = {
    nixpkgs.url = "github:flox/nixpkgs/stable";
    flox.url = "github:flox/flox/brantley/systemd-nixosModule"; # temporary, see note above
  };

  outputs =
    {
      self,
      nixpkgs,
      ...
    }@inputs:
    {
      # TODO: replace "my-nixos" in the next line with your hostname
      nixosConfigurations.my-nixos = nixpkgs.lib.nixosSystem rec {
        # TODO: replace "x86_64-linux" with your system type
        system = "x86_64-linux";
        specialArgs = { inherit system; };
        modules = [
          ./configuration.nix
          inputs.flox.nixosModules.flox
        ];
      };
    };
}
```

Once completed, rebuild and switch into your updated system configuration with the following:
```bash
nixos-rebuild switch
```

### Enable Flox

With the Flox flake included in your system configuration
you can now download pre-built binaries from cache.flox.dev,
which makes it much faster to enable Flox
by adding the following line to `/etc/nixos/configuration.nix`:

```nix
  programs.flox.enable = true;
```

Again as before, deploy the change using the following command:
```bash
nixos-rebuild switch
```

## Usage

NixOS modules provides a rich interface for
modeling configuration options for services,
setting required environment variables
and communicating various settings to related services.

The Flox NixOS module uses this functionality
to invoke services from applications provided by Flox environments,
with facilities for
automatically updating environments at service startup
and refreshing environments at periodic intervals.

There are two ways of configuring systemd services to run from Flox environments:

1. **Flox Services**.
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

2. **Flox Overrides**.
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

## Appendix: Flox NixOS module configuration attributes

### Common configuration attributes

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

### Flox Overrides configuration attributes

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
