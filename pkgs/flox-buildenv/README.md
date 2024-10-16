# Flox environment builder

The files in this directory modify the files in nixpkgs:pkgs/build-support/buildenv
to build fully-recursively-linked environment packages, complete with the flox
`activation-scripts` package.

To test:

```
nix build .#flox-buildenv
result/bin/buildenv \
  -n <name> \
  -a <flox-activation-scripts-package-path> \
  <path/to/manifest.json>
```
