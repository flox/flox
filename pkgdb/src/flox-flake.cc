/* ========================================================================== *
 *
 * @file flox-flake.cc
 *
 * @brief Defines a convenience wrapper that provides various operations on
 *        a `flake`.
 *
 *
 * -------------------------------------------------------------------------- */

#include <cassert>
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
callInChildProcess( std::function<void()>  lambda,
                    const std::exception & thrownOnError )
{
  pid_t pid = fork();
  if ( pid == -1 )
    {
      errorLog( "callInChildProcess: failed to fork!" );
      exit( EXIT_FAILURE );
    }
  if ( 0 < pid )
    {
      debugLog( nix::fmt( "callInChildProcess: waiting for child: %d", pid ) );
      int status = 0;
      waitpid( pid, &status, 0 );
      debugLog( nix::fmt(
        "callInChildProcess: child is finished, exit code: %d, signal: %d",
        WEXITSTATUS( status ),
        WTERMSIG( status ) ) );

      if ( WIFEXITED( status ) )
        {
          if ( WEXITSTATUS( status ) == EXIT_SUCCESS )
            {
              /* Success */
              return;
            }
          /* The error has already been reported via the child, just pass
           * along the exit code. */
          exit( WEXITSTATUS( status ) );
        }
      else { throw thrownOnError; }
    }
  else
    {
      lambda();
      try
        {
          debugLog( "callInChildProcess(child): finished, exiting" );
          exit( EXIT_SUCCESS );
        }
      catch ( const std::exception & err )
        {
          debugLog(
            nix::fmt( "callInChildProcess(child): caught exception on exit: %s",
                      err.what() ) );
          exit( EXIT_SUCCESS );
        }
    }
}

nix::flake::LockedFlake
lockFlake( nix::EvalState &              state,
           const nix::FlakeRef &         flakeRef,
           const nix::flake::LockFlags & lockFlags )
{
  auto nixLockFlake
    = [&]() { return nix::flake::lockFlake( state, flakeRef, lockFlags ); };
  // Calling this in a child process will ensure downloads are complete,
  // keeping file transfers isolated to a child process.
  callInChildProcess( nixLockFlake,
                      LockFlakeException( "failed to lock flake" ) );
  return ( nixLockFlake() );
}


FloxFlake::FloxFlake( const nix::ref<nix::EvalState> & state,
                      const nix::FlakeRef &            ref )
try : state( state ),
  lockedFlake( flox::lockFlake( *this->state, ref, defaultLockFlags ) )
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
