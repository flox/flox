# Fixed-output derivation used as a stable store path in publish mock tests.
#
# Because fixed-output derivations are content-addressed (by outputHash),
# this store path is byte-stable across machines and nixpkgs revisions:
#   /nix/store/xfigz788kjqvyyxdnyvycs0bfc6cdjp3-cli-128-fixed-empty
# That is what lets the path be embedded in the recorded publish mock bodies.
#
# The dev shell realises this derivation and exports the resulting path as
# FLOX_TEST_FIXED_STORE_PATH, so tests never build anything themselves.
#
# To build manually: nix-build --no-out-link test_data/manually_generated/cli-128-fixed-empty.nix
{
  system ? builtins.currentSystem,
}:
derivation {
  name = "cli-128-fixed-empty";
  inherit system;
  builder = "/bin/sh";
  args = [
    "-c"
    "echo cli128 > $out"
  ];
  outputHashAlgo = "sha256";
  outputHash = "sha256-bu6MtKc2APyhK/ltUbl1StrFDn2ZfBHWnbtm3whPSn8=";
  outputHashMode = "flat";
}
