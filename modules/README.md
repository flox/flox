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

1. add `flox.url = "github:flox/flox"` to `inputs`
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
    flox.url = "github:flox/flox";
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

With the Flox flake installed
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

The Flox NixOS module taps into this functionality
to invoke services from applications provided by Flox environments,
with facilities for
automatically updating environments at service startup
and refreshing environments at periodic intervals.

### Flox NixOS module configuration args

Following are the configuration options supported by the Flox module:

Mandatory attributes:

* `environment`
    The Flox environment to use for the service.

    - _Type_: string
    - _Example_: "flox/default"

Optional attributes:

* `execStart`
    The command to override the unit's ExecStart with.

    - _Type_: null or string
    - _Default_: `null`
    - _Example_: "flox/default"

* `script`
    The command to override the unit's script with.

    - _Type_: null or string
    - _Default_: string
    - _Example_: "flox/default"

* `floxHubTokenFile`
    Full path to the FloxHub token file.

    - _Type_: null or path
    - _Default_: `null`
    - _Example_: "flox/default"

* `extraFloxArgs`
    Additional arguments to pass to `flox`.

    - _Type_: list of strings
    - _Default_: [ ]
    - _Example_: "-v -v"

* `extraFloxActivateArgs`
    Additional arguments to pass to `flox activate`.

    - _Type_: list of strings
    - _Default_: [ ]
    - _Example_: "--mode dev"

* `extraFloxPullArgs`
    Additional arguments to pass to `flox pull`.

    - _Type_: list of strings
    - _Default_: [ ]
    - _Example_: "flox/default"

* `pullAtServiceStart`
    Whether to pull the Flox environment at service start.

    - _Type_: string
    - _Default_: `null`
    - _Example_: "flox/default"

* `floxServiceManager`
    Whether to use the internal Flox service management.

    - _Type_: string
    - _Default_: `null`
    - _Example_: "flox/default"

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

## Overriding a service

Enabling Flox by setting `programs.flox.enable = true;` will add an attribute to `systemd.services.*` called `flox`.
This allows you to override systemd services to use Flox environments.

## Examples

### Directly overriding a service not defined by a NixOS module

```nix
{
  pkgs,
  flox,
  floxServiceAttrs,
  ...
}:
{
    programs.flox = {
        enable = true;
        package = flox.packages.${pkgs.system}.flox;
    };

    systemd.services.floxEM = {
      wantedBy = ["multi-user.target"];
      requires = ["floxEM-setup.service"];
      after = ["floxEM-setup.service"];
      serviceConfig =
        {
          SyslogIdentifier = "floxEM";
          EnvironmentFile = cfg.environmentFile;
          User = cfg.user;
          WorkingDirectory = cfg.dataDir;
          ExecStart = "${pkgs.floxEM}/bin/floxEM --bind ${cfg.host}:${toString cfg.port}"; # (1)
          Restart = "on-failure";
        }
      environment = env;
    }
    // lib.optionalAttrs (cfg.envName == "production") {
      flox = { # (2)
        inherit (floxServiceAttrs) environment floxHubTokenFile; # (3)
        execStart = "floxEM --bind ${cfg.host}:${toString cfg.port}"; # (4)
      };
    };
}
```

1. Note that we are still defining an `ExecStart` attribute. This is the part we'll be overriding later.
1. Since we enabled Flox with `programs.flox.enable = true;`, we can now use the `flox` attribute within the `systemd.services.*` attributes.
1. We get the Flox environment and a floxHubTokenFile from elsewhere. The `environment` attribute has the format of Flox environments. The `floxHubTokenFile` attribute—at the time of writing—is the path to a GitHub access token.
1. Here, we override the `ExecStart` attribute to use another command than previously defined.

### Overriding a service defined by a NixOS module

```nix
{
  flox,
  ...
}: {
  programs.flox = {
    enable = true; # (1)
    package = flox.packages.${pkgs.system}.flox;
  };
  services.cowsay = {
    enable = true; # (2)
    interval = "*-*-* *:*:00,10";
    message = "Moo2!";
  };
  systemd.services.cowsay.flox = { # (3)
    environment = "foobar/cowsay"; # (4)
    execStart = "cowsay ${config.services.cowsay.message}"; # (5)
    floxHubTokenFile = "/run/secrets/cowsay/github.token";
  };
}
```

1. We enable Flox, this installs Flox system-wide.
1. We enable the cowsay NixOS service, which creates the `systemd.services.cowsay` attribute.
1. We now override the necessary attributes to start using Flox environments.
1. The Flox environment to use.
1. The command to run.

