/* ========================================================================== *
 *
 * @file main.cc
 *
 * @brief Executable exposing CRUD operations for package metadata.
 *
 *
 * -------------------------------------------------------------------------- */

#include <cstdlib>
#include <exception>
#include <iostream>
#include <map>
#include <stdexcept>
#include <string>
#include <string_view>
#include <unistd.h>

#include <nix/error.hh>
#include <nix/eval.hh>
#include <nix/shared.hh>
#include <nix/store-api.hh>

#include <nix/util.hh>


#include "flox/buildenv/command.hh"
#include "flox/buildenv/realise.hh"
#include "flox/core/exceptions.hh"
#include "flox/core/util.hh"
#include "flox/eval.hh"
#include "flox/lock-flake-installable.hh"
#include "flox/pkgdb/metrics.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

#ifndef NIXPKGS_CACERT_BUNDLE_CRT
#  error "NIXPKGS_CACERT_BUNDLE_CRT must be set"
#endif

/* -------------------------------------------------------------------------- */

/**
 * @class CaughtException
 * @brief An exception thrown when an otherwise unhandled exception is caught.
 *        This ensures proper JSON formatting.
 * @{
 */
FLOX_DEFINE_EXCEPTION( CaughtException,
                       EC_FAILURE,
                       "caught an unhandled exception" )
/** @} */


/* -------------------------------------------------------------------------- */

/**
 * @class NixException
 * @brief An exception thrown when an otherwise unhandled Nix exception is
 *        caught. This ensures proper JSON formatting.
 * @{
 */
FLOX_DEFINE_EXCEPTION( NixException, EC_NIX, "caught a nix exception" )
/** @} */


/* -------------------------------------------------------------------------- */

}  // namespace flox

/* -------------------------------------------------------------------------- */

void
setVerbosityFromEnv()
{
  auto * valueChars = std::getenv( "_FLOX_PKGDB_VERBOSITY" );
  if ( valueChars == nullptr ) { return; }
  std::string value( valueChars );
  if ( value == std::string( "-1" ) || value == std::string( "0" ) )
    {
      nix::verbosity = nix::lvlError;
    }
  else if ( value == std::string( "1" ) ) { nix::verbosity = nix::lvlInfo; }
  else if ( value == std::string( "2" ) ) { nix::verbosity = nix::lvlDebug; }
  else if ( value == std::string( "3" ) ) { nix::verbosity = nix::lvlChatty; }
  else if ( value == std::string( "4" ) ) { nix::verbosity = nix::lvlVomit; }
  // Put this at the end so that if we *want* logging it will show up
  traceLog( "found _FLOX_PKGDB_VERBOSITY=" + value );
}


/* -------------------------------------------------------------------------- */

int
run( int argc, char * argv[] )
{

  /* Define arg parsers. */

  flox::command::VerboseParser prog( "pkgdb", FLOX_PKGDB_VERSION );
  prog.add_description( "CRUD operations for package metadata" );

  flox::LockFlakeInstallableCommand cmdLock;
  prog.add_subparser( cmdLock.getParser() );

  flox::buildenv::BuildEnvCommand cmdBuildEnv;
  prog.add_subparser( cmdBuildEnv.getParser() );

  // used for bats tests, may be redundantly covered through
  // tests in `buildnev.rs` or can be reimplmented there.
  flox::EvalCommand cmdEval;
  prog.add_subparser( cmdEval.getParser() );

  /* Parse Args */
  try
    {
      prog.parse_args( argc, argv );
    }
  catch ( const std::runtime_error & err )
    {
      throw flox::command::InvalidArgException( err.what() );
    }

  /* Set the verbosity level requested by flox */
  setVerbosityFromEnv();

  // We wait to init here so we have verbosity.
  flox::sentryReporting.init( nix::verbosity >= nix::lvlDebug );

  // Initialize nix (and load config);
  nix::initNix();
  nix::initGC();

  if ( const char * rawStoreURI = std::getenv( "_FLOX_NIX_STORE_URL" ) )
    {
      auto providedStoreURI  = std::string( rawStoreURI );
      nix::settings.storeUri = providedStoreURI;
    }
  auto store = nix::openStore();
  auto state = nix::make_ref<nix::EvalState>( nix::SearchPath(), store, store );


  /* Run subcommand */
  if ( prog.is_subcommand_used( cmdLock.getParser() ) )
    {
      return cmdLock.run( state );
    }
  else if ( prog.is_subcommand_used( cmdBuildEnv.getParser() ) )
    {
      return cmdBuildEnv.run( state );
    }
  else if ( prog.is_subcommand_used( cmdEval.getParser() ) )
    {
      return cmdEval.run( *state );
    }

  // TODO: better error for this,
  // likely only occurs if we add a new command without handling it (?)
  throw flox::FloxException( "unrecognized command" );
}

/* -------------------------------------------------------------------------- */
int
printAndReturnException( const flox::FloxException & err )
{
  if ( isatty( STDOUT_FILENO ) == 0 )
    {
      std::cout << nlohmann::json( err ).dump() << '\n';
    }
  else { std::cerr << err.what() << '\n'; }

  return err.getErrorCode();
}

/* -------------------------------------------------------------------------- */

int
main( int argc, char * argv[] )
{
  /* Allows you to run without catching which is useful for
   * `gdb'/`lldb' backtraces. */
  auto * maybeNC = std::getenv( "PKGDB_NO_CATCH" );
  if ( maybeNC != nullptr )
    {
      std::string noCatch = std::string( maybeNC );
      if ( ( maybeNC != std::string( "" ) )
           && ( maybeNC != std::string( "0" ) ) )
        {
          return run( argc, argv );
        }
    }

  // Required to download flakes, but don't override if already set.
  setenv( "NIX_SSL_CERT_FILE", NIXPKGS_CACERT_BUNDLE_CRT, 0 );

  /* Wrap all execution in an error handler that pretty prints exceptions. */
  int exit_code = 0;
  try
    {
      exit_code = run( argc, argv );
    }
  catch ( const flox::FloxException & err )
    {
      exit_code = printAndReturnException( err );
    }
  // TODO: we may want to catch these closer to where they are
  //       originally thrown.
  // TODO: handle IFD build errors.
  catch ( const nix::Error & err )
    {
      exit_code = printAndReturnException(
        flox::NixException( "running pkgdb subcommand",
                            nix::filterANSIEscapes( err.what(), true ) ) );
    }
  catch ( const std::exception & err )
    {
      exit_code = printAndReturnException(
        flox::CaughtException( "running pkgdb subcommand", err.what() ) );
    }

  flox::sentryReporting.shutdown();

  return exit_code;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
