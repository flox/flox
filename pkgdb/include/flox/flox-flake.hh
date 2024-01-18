/* ========================================================================== *
 *
 * @file flox/flox-flake.hh
 *
 * @brief Defines a convenience wrapper that provides various operations on
 *        a `flake`.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <memory>
#include <nix/eval.hh>
#include <nix/flake/flake.hh>
#include <vector>

#include <nlohmann/json.hpp>

#include "flox/core/exceptions.hh"
#include "flox/core/nix-state.hh"
#include "flox/core/types.hh"
#include "flox/core/util.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/**
 * Default flags used when locking flakes.
 * - Disable `updateLockFile` and read existing lockfiles directly.
 * - Disable `writeLockFile` to avoid writing generated lockfiles to the
 *   filesystem; this will only occur if there is no existing lockfile.
 */
static const nix::flake::LockFlags defaultLockFlags
  = { .recreateLockFile = false /* default */
      ,
      .updateLockFile = false,
      .writeLockFile  = false,
      .useRegistries  = false,
      /* Remaining fields are defaults */
      .applyNixConfig        = false,
      .allowUnlocked         = true,
      .commitLockFile        = false,
      .referenceLockFilePath = std::nullopt,
      .outputLockFilePath    = std::nullopt,
      .inputOverrides        = {},
      .inputUpdates          = {} };

static const nix::flake::LockFlags floxFlakeLockFlags
  = { .recreateLockFile = false /* default */
      ,
      .updateLockFile = true,
      .writeLockFile  = true,
      .useRegistries  = false,
      /* Remaining fields are defaults */
      .applyNixConfig        = false,
      .allowUnlocked         = true,
      .commitLockFile        = false,
      .referenceLockFilePath = std::nullopt,
      .outputLockFilePath    = std::nullopt,
      .inputOverrides        = {},
      .inputUpdates          = {} };


/* -------------------------------------------------------------------------- */

/**
 * @brief Lock a flake so that evaluations may be cached in a SQL database.
 *
 * This is a lightweight wrapper over `nix::flake::lockFlake` with improved
 * error messaging.
 */
nix::flake::LockedFlake
lockFlake( nix::EvalState &              state,
           const nix::FlakeRef &         ref,
           const nix::flake::LockFlags & flags = defaultLockFlags );


/* -------------------------------------------------------------------------- */

/** @brief Load a flake's root values into a `nix` evaluator's state. */
[[nodiscard]] nix::Value *
flakeLoader( nix::EvalState &                state,
             const nix::flake::LockedFlake & lockedFlake );


/* -------------------------------------------------------------------------- */

/** @brief Open a `nix::eval_cache::EvalCache` for a locked flake. */
nix::ref<nix::eval_cache::EvalCache>
openEvalCache( nix::EvalState &                state,
               const nix::flake::LockedFlake & lockedFlake );

/* -------------------------------------------------------------------------- */

/**
 * @brief A convenience wrapper that provides various operations on a `flake`.
 *
 * Notably this class is responsible for a `nix` `EvalState` and an
 * `EvalCache` database associated with a `flake`.
 *
 * It is recommended that only one `FloxFlake` be created for a unique `flake`
 * to avoid synchronization slowdowns with its databases.
 */
class FloxFlake : public std::enable_shared_from_this<FloxFlake>
{

protected:

  /**
   * @brief A handle for a cached `nix` evaluator associated with @a this flake.
   * This is opened lazily by @a openEvalCache and remains open until @a this
   * object is destroyed.
   */
  std::shared_ptr<nix::eval_cache::EvalCache> _cache;

public:

  nix::ref<nix::EvalState>      state;
  const nix::flake::LockedFlake lockedFlake;

  FloxFlake( const nix::ref<nix::EvalState> & state,
             nix::flake::LockedFlake          lockedFlake )
    : state( state ), lockedFlake( std::move( lockedFlake ) )
  {}

  // FloxFlake( nix::ref<nix::EvalState> & state, const nix::FlakeRef & ref );
  FloxFlake( const nix::ref<nix::EvalState> & state,
             const nix::FlakeRef &            ref );

  /**
   * @brief Open a `nix` evaluator ( with an eval cache when possible ) with the
   * evaluated `flake` and its outputs in global scope.
   * @return A `nix` evaluator, potentially with caching.
   */
  [[nodiscard]] nix::ref<nix::eval_cache::EvalCache>
  openEvalCache();

  /**
   * @brief Try to open a `nix` evaluator cursor at a given path.
   * If there is no such attribute this routine will return `nullptr`.
   * @param path The attribute path try opening.
   * @return `nullptr` iff there is no such path, otherwise a
   *         @a nix::eval_cache::AttrCursor at @a path.
   */
  [[nodiscard]] MaybeCursor
  maybeOpenCursor( const AttrPath & path );

  /**
   * @brief Open a `nix` evaluator cursor at a given path.
   * If there is no such attribute this routine will throw an error.
   * @param path The attribute path to open.
   * @return A @a nix::eval_cache::AttrCursor at @a path.
   */
  [[nodiscard]] Cursor
  openCursor( const AttrPath & path );

}; /* End class `FloxFlake' */

/* -------------------------------------------------------------------------- */

/**
 * @class flox::LockFlakeException
 * @brief An exception thrown when locking a flake
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( LockFlakeException,
                       EC_NIX_LOCK_FLAKE,
                       "error locking flake" )
/** @} */


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
