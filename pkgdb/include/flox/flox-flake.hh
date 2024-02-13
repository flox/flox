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
#include <sys/wait.h>
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
 * A convenience wrapper that provides various operations on a `flake`.
 *
 * Notably this class is responsible for a `nix` `EvalState` and an
 * `EvalCache` database associated with a `flake`.
 *
 * It is recommended that only one `FloxFlake` be created for a unique `flake`
 * to avoid synchronization slowdowns with its databases.
 */
class FloxFlake : public std::enable_shared_from_this<FloxFlake>
{

private:

  /**
   * A handle for a cached `nix` evaluator associated with @a this flake.
   * This is opened lazily by @a openEvalCache and remains open until @a this
   * object is destroyed.
   */
  std::shared_ptr<nix::eval_cache::EvalCache> _cache;

public:

  nix::ref<nix::EvalState>      state;
  const nix::flake::LockedFlake lockedFlake;

  FloxFlake( const nix::ref<nix::EvalState> & state,
             const nix::FlakeRef &            ref );

  /**
   * Open a `nix` evaluator ( with an eval cache when possible ) with the
   * evaluated `flake` and its outputs in global scope.
   * @return A `nix` evaluator, potentially with caching.
   */
  nix::ref<nix::eval_cache::EvalCache>
  openEvalCache();

  /**
   * Try to open a `nix` evaluator cursor at a given path.
   * If there is no such attribute this routine will return `nullptr`.
   * @param path The attribute path try opening.
   * @return `nullptr` iff there is no such path, otherwise a
   *         @a nix::eval_cache::AttrCursor at @a path.
   */
  MaybeCursor
  maybeOpenCursor( const AttrPath & path );

  /**
   * Open a `nix` evaluator cursor at a given path.
   * If there is no such attribute this routine will throw an error.
   * @param path The attribute path to open.
   * @return A @a nix::eval_cache::AttrCursor at @a path.
   */
  Cursor
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

void
ensureFlakeIsDownloaded( auto && lambda )
{
  pid_t pid = fork();
  if ( pid == -1 )
    {
      // WML - TODO - better error handling here!
      errorLog( nix::fmt(
        "ensureFlakeIsDownloaded: faild to fork for flake downlod!" ) );
      exit( -1 );
    }
  if ( 0 < pid )
    {
      debugLog(
        nix::fmt( "ensureFlakeIsDownloaded: waiting for child:%d", pid ) );
      int status = 0;
      waitpid( pid, &status, 0 );
      debugLog(
        nix::fmt( "ensureFlakeIsDownloaded: child is finished, exitcode:%d",
                  status ) );

      if ( WEXITSTATUS( status ) == EXIT_SUCCESS )
        {
          // The flake should be downloaded and cached locally now
          return;
        }
      else
        {
          // what to do here?  The error has already been reported via the
          // child!
          exit( WEXITSTATUS( status ) );
        }
    }
  else
    {
      lambda();
      try
        {
          exit( EXIT_SUCCESS );
        }
      catch ( const std::exception & err )
        {
          debugLog( nix::fmt(
            "ensureFlakeIsDownloaded(child): caught exception on exit: %s",
            err.what() ) );
          exit( EXIT_SUCCESS );
        }
    }
}

/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
