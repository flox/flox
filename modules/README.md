# NixOS, nix-darwin and home-manager Flox module

Flox modules provide a convenient way of installing the Flox CLI
and configuring the necessary Nix system-wide settings in one place.

Installing the Flox module is a simple matter of importing a module to your
NixOS, nix-darwin or home-manager configuration.

Please refer to the appropriate section below depending on installation
requirements.


## NixOS

### Without flake

Download or clone `flox` repository to your system.

```console
$ cd /etc/nixos
$ git clone https://github.com/flox/flox
```

In your current machine configuration (most likely the machine configuration is
at `/etc/nixos/configuration.nix`) enable Flox by adding the following line:

```nix
{...}: {

  imports = [
    /etc/nixos/flox/modules/nixos.nix
  ]

  # The rest of your NixOS configuration
}
```

Then run `nixos-rebuild` as usual, but **only for the first time** provide
custom binary cache to avoid building Flox, eg:

```console
$ nixos-rebuild switch \
  --option extra-substituters "https://cache.flox.dev" \
  --option extra-trusted-public-keys "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
```


### With flake

With flakes you don't need to download/clone any sources since flakes manages
this for you.

Here is an example `flake.nix` which and how Flox module is imported:

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
      flox,
    }:
    {
      nixosConfigurations.my-machine = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          flox.nixosModules.flox
          ./configuration.nix
        ];
      };
    };
}
```

Then run `nixos-rebuild` command as usual. In our example we would run:

```console
$ nixos-rebuild switch --flake .#my-machine \
  --option extra-substituters "https://cache.flox.dev" \
  --option extra-trusted-public-keys "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
```


## nix-darwin

An example `flake.nix` that imports `flox` module would look like:

```nix
{
  inputs = {
    nixpkgs.url = "github:flox/nixpkgs/stable";
    nix-darwin.url = "github:lnl7/nix-darwin";
    flox.url = "github:flox/flox";
  };

  outputs =
    {
      self,
      nixpkgs,
      nix-darwin,
      flox,
    }:
    {
      darwinConfigurations.my-machine = nix-darwin.lib.darwinSystem {
        system = "aarch64-darwin";
        modules = [
          flox.darwinModules.flox
          ./darwin-configuration.nix
        ];
      };
    };
}
```

Then run `darwin-rebuild` command as usual. In our example we would run:

```console
$ darwin-rebuild switch --flake .#my-machine \
  --option extra-substituters "https://cache.flox.dev" \
  --option extra-trusted-public-keys "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
```


## Home Manager

An example `flake.nix` that imports `flox` module would look like:

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
      nix-darwin,
      flox,
    }:
    {
      homeConfigurations.my-machine = home-manager.lib.homeManagerConfiguration {
        pkgs = import nixpkgs { inherit system; };
        modules = [
          flox.homeModules.flox
          ./home-configuration.nix
        ];
      };
    };
}
```

Then run `home-manager` command as usual. In our example we would run:

```console
$ home-manager switch --flake .#my-machine \
  --option extra-substituters "https://cache.flox.dev" \
  --option extra-trusted-public-keys "flox-cache-public-1:7F4OyH7ZCnFhcze3fJdfyXYLQw/aV7GEed86nQ7IsOs="
```
