# Flox environment builder

This directory is derived from the files in nixpkgs:pkgs/build-support/buildenv,
modified to build multiple environment outputs:

* `runtime`: the same env as created by nixpkgs `buildenv` upstream
* `develop`: as above, but recurses to include links for all requisite packages
* `build-<pname>`: for each manifest build, creates env referring only to
  the "toplevel" packages, optionally filtered by `runtime-packages`

The Flox environment builder also includes logic to automatically add the
following packages to each output:

* the flox "interpreter": also known as "etc-profiles", this package includes
  scripts and requisites required to activate a flox environment
* the "manifest" package: this package contains `manifest.lock` and other files
  derived from that data (e.g. the `activate.d/envrc` file)

## Building and testing

You can build the `flox-buildenv` package as a flake and test it by
way of its usage hints as shown below.

```
% nix build -o result-buildenv .#flox-buildenv
% result-buildenv/bin/buildenv -h
Usage: result-buildenv/bin/buildenv [-x] \
  [-n <name>] \
  [-s <path/to/service-config.yaml>] \
  <path/to/manifest.lock>
-x : Enable debugging output.
-n <name> : The name of the flox environment to render.
-s <path> : Path to the service configuration file.
% result-buildenv/bin/buildenv $FLOX_ENV/manifest.lock | jq .
It took 0.061 seconds to realise the packages with pkgdb.
[
  {
    "drvPath": "/nix/store/qvv16w2mig6nwr7wdrn3ymm6dvwi9ci9-environment.drv",
    "outputs": {
      "develop": "/nix/store/kqzy26j4ldsd0az4sbbpc0yd41b2rfvw-environment-develop",
      "runtime": "/nix/store/warx31v9gmjadjdgdx2bskk1yr2wxl22-environment-runtime"
    }
  }
]
It took 0.038 seconds to render the flox environment outputs as Nix packages.
%
```
