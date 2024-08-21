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
 * Default flags used when locking flakes.
 * - Disable `updateLockFile` and read existing lockfiles directly.
 * - Disable `writeLockFile` to avoid writing generated lockfiles to the
 *   filesystem; this will only occur if there is no existing lockfile.
 */
static const nix::flake::LockFlags defaultLockFlags = {
  .recreateLockFile = false /* default */
  ,
  .updateLockFile = false,
  .writeLockFile  = false,
  .useRegistries  = std::nullopt /* default */
  ,
  .applyNixConfig = false /* default */
  ,
  .allowUnlocked = true /* default */
  ,
  .commitLockFile = false /* default */
  ,
  .referenceLockFilePath = std::nullopt /* default */
  ,
  .outputLockFilePath = std::nullopt /* default */
  ,
  .inputOverrides = {} /* default */
  ,
  .inputUpdates = {} /* default */
};

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
prim_getFingerprint( nix::EvalState & state,
                     nix::PosIdx      pos,
                     nix::Value **    args,
                     nix::Value &     value );

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
