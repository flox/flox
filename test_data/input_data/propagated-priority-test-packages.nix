# Test packages for the propagated dependency priority regression test (#2958).
#
# - python3-explicit:  provides bin/python3 (installed explicitly)
# - propagator:        propagates python3-propagated which also provides
#                      bin/python3 via nix-support/propagated-build-inputs
#
# Usage:
#   nix-build this-file.nix -A python3-explicit --arg coreutils /nix/store/...-coreutils --no-out-link
#   nix-build this-file.nix -A propagator      --arg coreutils /nix/store/...-coreutils --no-out-link
{ coreutils }:

let
  system = builtins.currentSystem;
  cu = builtins.storePath coreutils;
in {
  python3-explicit = derivation {
    name = "python3-explicit-test";
    inherit system;
    builder = "/bin/sh";
    PATH = "${cu}/bin";
    args = ["-c" ''
      mkdir -p $out/bin
      printf '#!/bin/sh\necho explicit\n' > $out/bin/python3
      chmod +x $out/bin/python3
    ''];
  };

  propagator = let
    propagated = derivation {
      name = "python3-propagated-test";
      inherit system;
      builder = "/bin/sh";
      PATH = "${cu}/bin";
      args = ["-c" ''
        mkdir -p $out/bin
        printf '#!/bin/sh\necho propagated\n' > $out/bin/python3
        chmod +x $out/bin/python3
      ''];
    };
  in derivation {
    name = "propagator-test";
    inherit system propagated;
    builder = "/bin/sh";
    PATH = "${cu}/bin";
    args = ["-c" ''
      mkdir -p $out/bin $out/nix-support
      printf '#!/bin/sh\necho propagator\n' > $out/bin/propagator-tool
      chmod +x $out/bin/propagator-tool
      echo $propagated > $out/nix-support/propagated-build-inputs
    ''];
  };
}
