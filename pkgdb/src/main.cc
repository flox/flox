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
#include <nix/util.hh>
#include <nlohmann/json.hpp>

#include "flox/buildenv/command.hh"
#include "flox/core/command.hh"
#include "flox/core/exceptions.hh"
#include "flox/eval.hh"
#include "flox/parse/command.hh"
#include "flox/pkgdb/command.hh"
#include "flox/pkgdb/metrics.hh"
#include "flox/repl.hh"
#include "flox/resolver/command.hh"
#include "flox/search/command.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

static sentryReporting theSentryReporting = sentryReporting();

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
  auto valueChars = std::getenv( "_FLOX_PKGDB_VERBOSITY" );
  if ( valueChars == nullptr ) { return; }
  std::string value( valueChars );
  if ( value == std::string( "0" ) ) { nix::verbosity = nix::lvlError; }
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

  flox::pkgdb::ScrapeCommand cmdScrape;
  prog.add_subparser( cmdScrape.getParser() );

  flox::pkgdb::GetCommand cmdGet;
  prog.add_subparser( cmdGet.getParser() );

  flox::pkgdb::ListCommand cmdList;
  prog.add_subparser( cmdList.getParser() );

  flox::pkgdb::GCCommand cmdGC;
  prog.add_subparser( cmdGC.getParser() );

  flox::search::SearchCommand cmdSearch;
  prog.add_subparser( cmdSearch.getParser() );

  flox::resolver::ManifestCommand cmdManifest;
  prog.add_subparser( cmdManifest.getParser() );

  flox::parse::ParseCommand cmdParse;
  prog.add_subparser( cmdParse.getParser() );

  flox::ReplCommand cmdRepl;
  prog.add_subparser( cmdRepl.getParser() );

  flox::EvalCommand cmdEval;
  prog.add_subparser( cmdEval.getParser() );

  flox::buildenv::BuildEnvCommand cmdBuildEnv;
  prog.add_subparser( cmdBuildEnv.getParser() );


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

  flox::theSentryReporting.init( true );

  /* Run subcommand */
  if ( prog.is_subcommand_used( "scrape" ) ) { return cmdScrape.run(); }
  if ( prog.is_subcommand_used( "get" ) ) { return cmdGet.run(); }
  if ( prog.is_subcommand_used( "list" ) ) { return cmdList.run(); }
  if ( prog.is_subcommand_used( "gc" ) ) { return cmdGC.run(); }
  if ( prog.is_subcommand_used( "search" ) ) { return cmdSearch.run(); }
  if ( prog.is_subcommand_used( "manifest" ) ) { return cmdManifest.run(); }
  if ( prog.is_subcommand_used( "parse" ) ) { return cmdParse.run(); }
  if ( prog.is_subcommand_used( "repl" ) ) { return cmdRepl.run(); }
  if ( prog.is_subcommand_used( "eval" ) ) { return cmdEval.run(); }
  if ( prog.is_subcommand_used( "buildenv" ) ) { return cmdBuildEnv.run(); }

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

  /* Wrap all execution in an error handler that pretty prints exceptions. */
  try
    {
      return run( argc, argv );
    }
  catch ( const flox::FloxException & err )
    {
      return printAndReturnException( err );
    }
  // TODO: we may want to catch these closer to where they are
  //       originally thrown.
  // TODO: handle IFD build errors.
  catch ( const nix::Error & err )
    {
      return printAndReturnException(
        flox::NixException( "running pkgdb subcommand",
                            nix::filterANSIEscapes( err.what(), true ) ) );
    }
  catch ( const std::exception & err )
    {
      return printAndReturnException(
        flox::CaughtException( "running pkgdb subcommand", err.what() ) );
    }
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
