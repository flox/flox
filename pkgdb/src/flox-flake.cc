/* ========================================================================== *
 *
 * @file flox-flake.cc
 *
 * @brief Defines a convenience wrapper that provides various operations on
 *        a `flake`.
 *
 *
 * -------------------------------------------------------------------------- */

#include <assert.h>
#include <exception>
#include <functional>
#include <memory>
#include <optional>
#include <string>
#include <vector>

#include <nix/attr-set.hh>
#include <nix/config.hh>
#include <nix/eval-cache.hh>
#include <nix/eval-inline.hh> /**< for inline `allocValue', and `forceAttrs'. */
#include <nix/eval.hh>
#include <nix/flake/flake.hh>
#include <nix/flake/flakeref.hh>
#include <nix/nixexpr.hh>
#include <nix/ref.hh>
#include <nix/symbol-table.hh>
#include <nix/util.hh>
#include <nix/value.hh>

#include "flox/core/types.hh"
#include "flox/flox-flake.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

void
ensureFlakeIsDownloaded( std::function<void()> && lambda )
{
  pid_t pid = fork();
  if ( pid == -1 )
    {
      // WML - TODO - better error handling here!
      errorLog( "ensureFlakeIsDownloaded: failed to fork for flake download!" );
      exit( EXIT_FAILURE );
    }
  if ( 0 < pid )
    {
      debugLog(
        nix::fmt( "ensureFlakeIsDownloaded: waiting for child: %d", pid ) );
      int status = 0;
      waitpid( pid, &status, 0 );
      debugLog( nix::fmt(
        "ensureFlakeIsDownloaded: child is finished, exit code: %d, signal: %d",
        WEXITSTATUS( status ),
        WTERMSIG( status ) ) );

      if ( WIFEXITED( status ) )
        {
          if ( WEXITSTATUS( status ) == EXIT_SUCCESS )
            {
              /* The flake should be downloaded and cached locally now
               * return to the caller. */
              return;
            }
          else
            {
              /* The error has already been reported via the child, just pass
               * along the exit code. */
              exit( WEXITSTATUS( status ) );
            }
        }
      else { throw LockFlakeException( "failed to lock flake" ); }
    }
  else
    {
      lambda();
      try
        {
          debugLog( "ensureFlakeIsDownloaded(child): finished, exiting" );
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

nix::flake::LockedFlake
lockFlake( nix::EvalState &              state,
           const nix::FlakeRef &         ref,
           const nix::flake::LockFlags & flags )
{
  try
    {
      /* Force fetching and population of `nix` _fetcher cache_ to occur
       * in a child process.
       * This allows us to safely `fork` elsewhere from the parent without
       * leaving open file descriptors or various other connections. */
      ensureFlakeIsDownloaded(
        [&]() -> void { (void) nix::flake::lockFlake( state, ref, flags ); } );
      /* Attempting to lock a second time will not trigger a fetch because the
       * flake has already been downloaded and is present in our local cache. */
      return nix::flake::lockFlake( state, ref, flags );
    }
  catch ( const std::exception & err )
    {
      throw LockFlakeException( "failed to lock flake \"" + ref.to_string()
                                  + "\"",
                                nix::filterANSIEscapes( err.what(), true ) );
    }
  catch ( ... )
    {
      throw LockFlakeException( "failed to lock flake \"" + ref.to_string()
                                + "\"" );
    }
}


/* -------------------------------------------------------------------------- */

nix::Value *
flakeLoader( nix::EvalState &                state,
             const nix::flake::LockedFlake & lockedFlake )
{
  nix::Value * vFlake = state.allocValue();
  /* Evaluate the `outputs' function using `inputs' as arguments. */
  nix::flake::callFlake( state, lockedFlake, *vFlake );
  state.forceAttrs( *vFlake, nix::noPos, "while parsing cached flake data" );
  /* Overwrite the _global_ `outputs` symbol with the evaluated result.
   * This makes the original `outputs` function inaccessible. */
  nix::Attr * aOutputs
    = vFlake->attrs->get( state.symbols.create( "outputs" ) );
  assert( aOutputs != nullptr );
  return aOutputs->value;
}


/* -------------------------------------------------------------------------- */

/** @brief Open a `nix::eval_cache::EvalCache` for a locked flake. */
nix::ref<nix::eval_cache::EvalCache>
openEvalCache( nix::EvalState &                state,
               const nix::flake::LockedFlake & lockedFlake )
{
  auto fingerprint = lockedFlake.getFingerprint();
  auto useCache = std::make_optional<std::reference_wrapper<const nix::Hash>>(
    std::cref( fingerprint ) );

  /* Push current settings. */
  bool oldUseCache = nix::evalSettings.useEvalCache;
  bool oldPureEval = nix::evalSettings.pureEval;

  nix::evalSettings.useEvalCache.assign( true );
  nix::evalSettings.pureEval.assign( true );

  /* Loads a flake into the `nix` evaluator and a SQLite3 cache database. */
  auto cache = std::make_shared<nix::eval_cache::EvalCache>(
    useCache,
    state,
    [&state, &lockedFlake]() { return flakeLoader( state, lockedFlake ); } );

  /* Pop old settings. */
  nix::evalSettings.useEvalCache.assign( oldUseCache );
  nix::evalSettings.pureEval.assign( oldPureEval );

  return static_cast<nix::ref<nix::eval_cache::EvalCache>>( cache );
}


/* -------------------------------------------------------------------------- */

nix::ref<nix::eval_cache::EvalCache>
FloxFlake::openEvalCache()
{
  if ( this->_cache == nullptr )
    {
      this->_cache = flox::openEvalCache( *this->state, this->lockedFlake );
    }
  return static_cast<nix::ref<nix::eval_cache::EvalCache>>( this->_cache );
}


/* -------------------------------------------------------------------------- */

MaybeCursor
FloxFlake::maybeOpenCursor( const AttrPath & path )
{
  MaybeCursor cur = this->openEvalCache()->getRoot();
  for ( const auto & part : path )
    {
      cur = cur->maybeGetAttr( part );
      if ( cur == nullptr ) { break; }
    }
  return cur;
}


/* -------------------------------------------------------------------------- */

Cursor
FloxFlake::openCursor( const AttrPath & path )
{
  Cursor cur = this->openEvalCache()->getRoot();
  for ( const auto & part : path ) { cur = cur->getAttr( part ); }
  return cur;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
