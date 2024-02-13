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
#include <optional>
#include <string>
#include <vector>

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
      debugLog( nix::fmt(
        "ensureFlakeIsDownloaded: child is finished, exitcode:%d, signal: %d",
        WEXITSTATUS( status ),
        WTERMSIG( status ) ) );

      if ( WIFEXITED( status ) )
        {
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
      else { throw LockFlakeException( "failed to lock flake" ); }
    }
  else
    {
      lambda();
      try
        {
          debugLog(
            nix::fmt( "ensureFlakeIsDownloaded(child): finished, exiting" ) );
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

FloxFlake::FloxFlake( const nix::ref<nix::EvalState> & state,
                      const nix::FlakeRef &            ref )
try : state( state ),
  lockedFlake(
    [&]()
    {
      auto getFlake = [&]()
      { nix::flake::lockFlake( *this->state, ref, defaultLockFlags ); };
      ensureFlakeIsDownloaded( getFlake );
      return nix::flake::lockFlake( *this->state, ref, defaultLockFlags );
    }() )
  {}
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


/* -------------------------------------------------------------------------- */

nix::ref<nix::eval_cache::EvalCache>
FloxFlake::openEvalCache()
{
  if ( this->_cache == nullptr )
    {
      auto fingerprint = this->lockedFlake.getFingerprint();
      this->_cache     = std::make_shared<nix::eval_cache::EvalCache>(
        ( nix::evalSettings.useEvalCache && nix::evalSettings.pureEval )
              ? std::optional { std::cref( fingerprint ) }
              : std::nullopt,
        *this->state,
        [&]()
        {
          nix::Value * vFlake = this->state->allocValue();
          nix::flake::callFlake( *this->state, this->lockedFlake, *vFlake );
          this->state->forceAttrs( *vFlake,
                                   nix::noPos,
                                   "while parsing cached flake data" );
          nix::Attr * aOutputs
            = vFlake->attrs->get( this->state->symbols.create( "outputs" ) );
          assert( aOutputs != nullptr );
          return aOutputs->value;
        } );
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
