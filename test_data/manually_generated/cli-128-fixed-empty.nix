# Fixed-output derivation used as a stable store path in publish mock tests.
#
# Because fixed-output derivations are content-addressed (by outputHash),
# this store path is byte-stable across machines and nixpkgs revisions:
#   /nix/store/xfigz788kjqvyyxdnyvycs0bfc6cdjp3-cli-128-fixed-empty
#
# Tests build this on demand via ensure_fixed_test_store_path() in publish.rs
# whenever the path is absent from the local Nix store. No manual pre-build
# step is required.
#
# To build manually: nix-build --no-out-link test_data/manually_generated/cli-128-fixed-empty.nix
derivation {
  name = "cli-128-fixed-empty";
  system = builtins.currentSystem;
  builder = "/bin/sh";
  args = [ "-c" "echo cli128 > $out" ];
  outputHashAlgo = "sha256";
  outputHash = "sha256-bu6MtKc2APyhK/ltUbl1StrFDn2ZfBHWnbtm3whPSn8=";
  outputHashMode = "flat";
}
