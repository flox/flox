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

#include "flox/core/command.hh"
#include "flox/core/exceptions.hh"
#include "flox/eval.hh"
#include "flox/parse/command.hh"
#include "flox/pkgdb/command.hh"
#include "flox/repl.hh"
#include "flox/resolver/command.hh"
#include "flox/search/command.hh"


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


  /* Parse Args */

  try
    {
      prog.parse_args( argc, argv );
    }
  catch ( const std::runtime_error & err )
    {
      throw flox::command::InvalidArgException( err.what() );
    }

  /* Run subcommand */

  if ( prog.is_subcommand_used( "scrape" ) ) { return cmdScrape.run(); }
  if ( prog.is_subcommand_used( "get" ) ) { return cmdGet.run(); }
  if ( prog.is_subcommand_used( "list" ) ) { return cmdList.run(); }
  if ( prog.is_subcommand_used( "search" ) ) { return cmdSearch.run(); }
  if ( prog.is_subcommand_used( "manifest" ) ) { return cmdManifest.run(); }
  if ( prog.is_subcommand_used( "parse" ) ) { return cmdParse.run(); }
  if ( prog.is_subcommand_used( "repl" ) ) { return cmdRepl.run(); }
  if ( prog.is_subcommand_used( "eval" ) ) { return cmdEval.run(); }

  // TODO: better error for this,
  // likely only occurs if we add a new command without handling it (?)
  throw flox::FloxException( "unrecognized command" );
}


/* -------------------------------------------------------------------------- */

int
main( int argc, char * argv[] )
{

  try
    {
      return run( argc, argv );
    }
  catch ( const flox::FloxException & err )
    {
      if ( ! isatty( STDOUT_FILENO ) )
        {
          std::cout << nlohmann::json( err ).dump() << std::endl;
        }
      else { std::cerr << err.what() << std::endl; }

      return err.getErrorCode();
    }
  // TODO: we may want to catch these closer to where they are
  //       originally thrown.
  // TODO: handle IFD build errors.
  catch ( const nix::Error & err )
    {
      if ( ! isatty( STDOUT_FILENO ) )
        {
          nlohmann::json error = {
            { "exit_code", flox::EC_NIX },
            { "message", nix::filterANSIEscapes( err.what(), true ) },
          };
          std::cout << error << std::endl;
        }
      else { std::cerr << err.what() << std::endl; }

      return flox::EC_NIX;
    }
  catch ( const std::exception & err )
    {
      if ( ! isatty( STDOUT_FILENO ) )
        {
          nlohmann::json error = {
            { "exit_code", EXIT_FAILURE },
            { "message", err.what() },
          };
          std::cout << error << std::endl;
        }
      else { std::cerr << err.what() << std::endl; }

      return flox::EC_FAILURE;
    }
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
