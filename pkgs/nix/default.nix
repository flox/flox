# ============================================================================ #
#
# Applies patches to `nix' and fixes up public header `#includes'.
#
# Additionally there's a wonky spot where they
# `#include "nlohmann/json_fwd.hpp"' in `include/nix/json-impls.hh' which forces
# consumers to use `-I' instead of `-isystem' for `nlohmann_json' when compiling
# against `nix'.
# This fixes that issue too.
#
#
# ---------------------------------------------------------------------------- #
{
  nixVersions,
  stdenv,
}: let
  nixWithStdEnv = nixVersions.nix_2_17.override {stdenv = stdenv;};
in
  nixWithStdEnv.overrideAttrs (prev: {
    separateDebugInfo = false;
    dontStrip = true;

    # Apply patch files.
    patches =
      prev.patches
      ++ [
        (builtins.path {path = ./patches/nix-9147.patch;})
        (builtins.path {path = ./patches/multiple-github-tokens.2.13.2.patch;})
      ];

    postFixup = ''
      # Generate a `sed' pattern to fix up public header `#includes'.
      # All header names separated by '\|'.
      _patt="$( find "$dev/include/nix" -type f -name '*.h*' -printf '%P\|'; )";
      # Strip leading/trailing '\|'.
      _patt="''${_patt%\\|}";
      _patt="''${_patt#\\|}";
      _patt="s,#include \+\"\($_patt\)\",#include <nix/\1>,";
      # Perform the substitution.
      find "$dev/include/nix" -type f -name '*.h*' -print                        \
        |xargs sed -i                                                            \
                   -e "$_patt"                                                   \
                   -e 's,#include \+"\(nlohmann/json_fwd\.hpp\)",#include <\1>,';

      # Fixup `pkg-config' files.
      find "$dev" -type f -name '*.pc'                       \
        |xargs sed -i -e 's,\(-I\''${includedir}\)/nix,\1,'  \
                      -e 's,-I,-isystem ,';

      # Create `nix-fetchers.pc'.
      cat <<EOF > "$dev/lib/pkgconfig/nix-fetchers.pc"
      prefix=$out
      libdir=$out/lib
      includedir=$dev/include

      Name: Nix
      Description: Nix Package Manager
      Version: 2.17.1
      Requires: nix-store bdw-gc
      Libs: -L\''${libdir} -lnixfetchers
      Cflags: -isystem \''${includedir} -std=c++2a
      EOF
    '';
  })
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #

