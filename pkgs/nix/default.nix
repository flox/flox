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
{
  nixVersions,
  stdenv,
}:
let
  nixVersion = "stable";
in
nixVersions."${nixVersion}".overrideAttrs (prev: {
  # Necessary for compiling with debug symbols
  inherit stdenv;

  # Apply patch files.
  patches = prev.patches ++ [
    (builtins.path { path = ./patches/seekable_http.patch; })

    # Flox, or more specifically nix expression builds,
    # currently use nix fetchers to fetch shallow git clones of nixpkgs.
    # The git fetcher in Nix < 2.29,
    # was not consistently using the same cache dir for operations on shallow clones,
    # i.e. when computing cache path was determined inconsistently via
    # `getCachePath(<url>, getShallowAttr(input))`, and `getCachePath(<url>, false)`.
    #
    # The most easily observable effect were "fatal" git messages and Nix warnings,
    # as documented in <https://github.com/flox/flox/issues/3346>.
    # Whether it has further effect on the nixpkgs used, is unclear but possible.
    #
    # The bug was fixed in Nix >= 2.29 via <https://github.com/NixOS/nix/pull/12642>.
    # Since the nix version used here is still 2.28.3,
    # backport the patch until our nixpkgs ships with a patched distribution of nix.
    #
    # Note: remove for Nix >= v2.29
    (builtins.path { path = ./patches/pr_12642_libfetchers_git_shallow_clone_cache.patch; })

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
    Version: ${prev.version}
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
