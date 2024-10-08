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
# N.B. we select the "stable" revision in the full knowledge/expectation that
# this will change out from underneath us over time. We do this along with a
# commitment to continually build against the (flox) nixpkgs staging and
# unstable branches in CI so that we will notified when upcoming upgrades of
# nixVersions.stable upstream break our build.
#
# ---------------------------------------------------------------------------- #
{ nixVersions, stdenv }:
nixVersions.stable.overrideAttrs (prev: {
  # Necessary for compiling with debug symbols
  inherit stdenv;

  # Apply patch files.
  patches = prev.patches ++ [
    (builtins.path { path = ./patches/nix-9147.patch; })
    (builtins.path { path = ./patches/multiple-github-tokens.2.13.2.patch; })
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
    Version: stable
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
