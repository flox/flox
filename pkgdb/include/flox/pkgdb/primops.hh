/* ========================================================================== *
 *
 * @file flox/pkgdb/primops.hh
 *
 * @brief Extensions to `nix` primitive operations.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include "flox/core/nix-state.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/**
 * @brief Lookup a flake's _fingerprint_ hash.
 *
 * This hash uniquely identifies a revision of a locked flake.
 *
 * Takes a single argument `flakeRef`: Either an attribute set or string.
 *
 * @param state The `nix` evaluator's state.
 * @param pos The position ( file name and line/column numbers ) of the call.
 *            This is generally used for error reporting.
 * @param args The arguments to the primitive.
 * @param value An allocated `nix::Value` to store the result in.
 */
void
prim_getFingerprint( nix::EvalState &  state,
                     const nix::PosIdx pos,
                     nix::Value **     args,
                     nix::Value &      value );


/* -------------------------------------------------------------------------- */

// TODO
#if 0

/**
 * @brief Lookup packages in a `pkgdb` database.
 *
 * Takes the following arguments:
 * - `fingerprint`: _fingerprint_ hash for the target flake.
 * - `query`: An attribut set containing `flox::resolver::PkgQueryArgs`.
 *
 * Example:
 * ```
 * builtins.queryPackages ( builtins.getFingerprint "github:NixOS/nixpkgs" ) {
 *   name              = null;     # :: null|string
 *   pname             = "hello";  # :: null|string
 *   version           = null;     # :: null|string
 *   semver            = null;     # :: null|string
 *   partialMatch      = null;     # :: null|string
 *   partialNameMatch  = null;     # :: null|string
 *   pnameOrAttrName   = null;     # :: null|string
 *   relPath           = null;     # :: null|list of strings
 *   licenses          = null;     # :: null|list of strings
 *   allowBroken       = false;    # :: null|bool            ( default: false )
 *   allowUnfree       = true;     # :: null|bool            ( default: true )
 *   preferPreReleases = false;    # :: null|bool            ( default: false )
 *   subtrees          = null;     # :: null|list of strings
 *   systems           = null;     # :: null|list of strings ( default:
 * current-system )
 * }
 *
 * # => [6066]
 * ```
 *
 * @param state The `nix` evaluator's state.
 * @param pos The position ( file name and line/column numbers ) of the call.
 *            This is generally used for error reporting.
 * @param args The arguments to the primitive.
 * @param value An allocated `nix::Value` to store the result in.
 */
void
prim_queryPackages( nix::EvalState &  state,
                    const nix::PosIdx pos,
                    nix::Value **     args,
                    nix::Value &      value );


/* -------------------------------------------------------------------------- */

/**
 * @brief Get information about a package from a `pkgdb` database.
 *
 * Takes the following arguments:
 * - `fingerprint`: _fingerprint_ hash for the target flake.
 * - `rowId`: An unsigned integer representing the row id of the package.
 *
 * @param state The `nix` evaluator's state.
 * @param pos The position ( file name and line/column numbers ) of the call.
 *            This is generally used for error reporting.
 * @param args The arguments to the primitive.
 * @param value An allocated `nix::Value` to store the result in.
 */
void
prim_getPackage( nix::EvalState &  state,
                 const nix::PosIdx pos,
                 nix::Value **     args,
                 nix::Value &      value );


/* -------------------------------------------------------------------------- */

#endif  // 0  ( TODO )

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
