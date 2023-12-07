/* ========================================================================== *
 *
 * @file search/command.cc
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#include <cstdlib>
#include <filesystem>
#include <iostream>
#include <memory>
#include <optional>
#include <string>
#include <variant>
#include <vector>

#include <argparse/argparse.hpp>
#include <nix/ref.hh>
#include <nlohmann/json.hpp>

#include "flox/core/command.hh"
#include "flox/core/util.hh"
#include "flox/pkgdb/input.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/pkgdb/read.hh"
#include "flox/registry.hh"
#include "flox/resolver/environment.hh"
#include "flox/resolver/lockfile.hh"
#include "flox/resolver/manifest.hh"
#include "flox/search/command.hh"
#include "flox/search/params.hh"


/* -------------------------------------------------------------------------- */

namespace flox::search {

/* -------------------------------------------------------------------------- */

argparse::Argument &
SearchCommand::addSearchParamArgs( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "parameters" )
    .help( "search paramaters as inline JSON or a path to a file" )
    .metavar( "[PARAMS]" )
    .nargs( argparse::nargs_pattern::optional )
    .action(
      [&]( const std::string & params )
      {
        nlohmann::json searchParamsRaw = parseOrReadJSONObject( params );
        searchParamsRaw.get_to( this->params );
      } );
}


/* -------------------------------------------------------------------------- */

void
SearchCommand::addSearchQueryOptions( argparse::ArgumentParser & parser )
{
  parser.add_argument( "--name" )
    .help( "search for packages by exact `name' match." )
    .metavar( "NAME" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.name = arg; } );

  parser.add_argument( "--pname" )
    .help( "search for packages by exact `pname' match." )
    .metavar( "PNAME" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.pname = arg; } );

  parser.add_argument( "--version" )
    .help( "search for packages by exact `version' match." )
    .metavar( "VERSION" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.version = arg; } );

  parser.add_argument( "--semver" )
    .help( "search for packages by semantic version range matching." )
    .metavar( "RANGE" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.semver = arg; } );

  parser.add_argument( "--match" )
    .help( "search for packages by partially matching `pname', "
           "`description', or `attrName'." )
    .metavar( "MATCH" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.partialMatch = arg; } );

  parser.add_argument( "--match-name" )
    .help( "search for packages by partially matching `pname' or `attrName'." )
    .metavar( "MATCH" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.partialNameMatch = arg; } );

  parser.add_argument( "--dump-query" )
    .help( "print the generated SQL query and exit." )
    .nargs( 0 )
    .implicit_value( true )
    .action( [&]( const auto & ) { this->dumpQuery = true; } );
}


/* -------------------------------------------------------------------------- */

SearchCommand::SearchCommand() : parser( "search" )
{
  this->parser.add_description(
    "Search a set of flakes and emit a list satisfactory packages." );
  this->addGARegistryOption( this->parser );
  this->addSearchParamArgs( this->parser );
  this->addFloxDirectoryOption( this->parser );
  this->addSearchQueryOptions( this->parser );
}


/* -------------------------------------------------------------------------- */

void
SearchCommand::initEnvironment()
{
  /* Init global manifest. */

  if ( auto path = this->params.getGlobalManifestPath(); path.has_value() )
    {
      this->setGlobalManifestRaw( *path );
    }
  else if ( auto raw = this->params.getGlobalManifestRaw(); raw.has_value() )
    {
      this->setGlobalManifestRaw( *raw );
    }

  /* Init manifest. */

  if ( auto path = this->params.getManifestPath(); path.has_value() )
    {
      this->setManifestRaw( *path );
    }
  else
    {
      auto raw = this->params.getManifestRaw();
      this->setManifestRaw( raw );
    }

  /* Init lockfile . */

  if ( auto path = this->params.getLockfilePath(); path.has_value() )
    {
      this->setLockfileRaw( *path );
    }
  else if ( auto raw = this->params.getLockfileRaw(); raw.has_value() )
    {
      this->setLockfileRaw( *raw );
    }
}


/* -------------------------------------------------------------------------- */

int
SearchCommand::run()
{
  /* Initialize environment. */
  this->initEnvironment();

  pkgdb::PkgQueryArgs args = this->getEnvironment().getCombinedBaseQueryArgs();
  this->params.query.fillPkgQueryArgs( args );
  auto query         = pkgdb::PkgQuery( args );
  auto countPkgQuery = pkgdb::PkgQuery( args, { "COUNT(*)" } );
  std::cerr << "made the queries" << std::endl;
  if ( this->dumpQuery )
    {
      std::cout << query.str() << std::endl;
      return EXIT_SUCCESS;
    }
  std::cerr << "about to check whether there's a limit" << std::endl;
  if ( query.limit.has_value() )
    {
      auto                       resultCount = 0;
      std::vector<pkgdb::row_id> rowIds;
      for ( const auto & [name, input] :
            *this->getEnvironment().getPkgDbRegistry() )
        {
          std::cerr << "start of a count query" << std::endl;
          // Execute the query counting the number of results
          auto dbRO = input->getDbReadOnly();
          std::cerr << "got the ro-db" << std::endl;
          auto counts = countPkgQuery.execute( dbRO->db );
          std::cerr << "executed the query" << std::endl;
          auto numResults = counts[0];
          std::cerr << "got the result count" << std::endl;
          resultCount += numResults;
          std::cerr << "finished a count query" << std::endl;
        }
      // Emit the number of results as the first line
      nlohmann::json resultCountRecord = { { "result-count", resultCount } };
      std::cout << resultCountRecord << std::endl;
      std::cerr << "sent the count record" << std::endl;
      for ( const auto & [name, input] :
            *this->getEnvironment().getPkgDbRegistry() )
        {
          std::cerr << "about to check limit at top" << std::endl;
          if ( *query.limit == 0 ) { break; }
          auto dbRO = input->getDbReadOnly();
          for ( const auto & row : query.execute( dbRO->db ) )
            {
              std::cerr << "about to check limit in the loop" << std::endl;
              if ( *query.limit == 0 ) { break; }
              std::cout << input->getRowJSON( row ).dump() << std::endl;
              std::cerr << "decrementing the limit" << std::endl;
              *query.limit -= 1;
            }
        }
    }
  else
    {
      for ( const auto & [name, input] :
            *this->getEnvironment().getPkgDbRegistry() )
        {
          auto dbRO = input->getDbReadOnly();
          for ( const auto & row : query.execute( dbRO->db ) )
            {
              std::cout << input->getRowJSON( row ).dump() << std::endl;
            }
        }
    }
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::search


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
