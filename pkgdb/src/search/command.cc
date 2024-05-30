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
    .help( "search for packages by exact 'name' match." )
    .metavar( "NAME" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.name = arg; } );

  parser.add_argument( "--pname" )
    .help( "search for packages by exact 'pname' match." )
    .metavar( "PNAME" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.pname = arg; } );

  parser.add_argument( "--rel-path" )
    .help( "search for packages by exact 'relPath'." )
    .metavar( "DOT.SEPARATED.REL.PATH" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.relPath = splitAttrPath( arg ); } );

  parser.add_argument( "--version" )
    .help( "search for packages by exact 'version' match." )
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
    .help( "search for packages by partially matching 'pname', "
           "'description', or 'attrName'." )
    .metavar( "MATCH" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.partialMatch = arg; } );

  parser.add_argument( "--match-name" )
    .help( "search for packages by partially matching 'pname' or 'attrName'." )
    .metavar( "MATCH" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.partialNameMatch = arg; } );

  parser.add_argument( "--match-name-or-rel-path" )
    .help( "search for packages by partially matching 'pname' or '.' joined "
           "'relPath'." )
    .metavar( "MATCH" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.partialNameOrRelPathMatch = arg; } );

  parser.add_argument( "--dump-query" )
    .help( "print the generated SQL query and exit." )
    .nargs( 0 )
    .implicit_value( true )
    .action( [&]( const auto & ) { this->dumpQuery = true; } );

  parser.add_argument( "--query-limit" )
    .help( "set the query-limit for results" )
    .nargs( 1 )
    .action( [&]( const std::string & arg )
             { this->params.query.limit = std::atoi( arg.c_str() ); } );
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
  nlohmann::json queryJson;
  to_json( queryJson, args );
  debugLog( "performing search with query: " + queryJson.dump() );
  auto query = pkgdb::PkgQuery( args );
  if ( this->dumpQuery ) { std::cout << query.str() << std::endl; }

  /* Collect results from each input */
  auto                                            globalResultCount = 0;
  std::vector<std::vector<pkgdb::row_id>>         globallyFoundIds;
  std::vector<std::shared_ptr<pkgdb::PkgDbInput>> inputs;
  for ( const auto & [name, input] :
        *this->getEnvironment().getPkgDbRegistry() )
    {
      auto                       dbRO = input->getDbReadOnly();
      std::vector<pkgdb::row_id> thisInputIds;

      debugLog( "querying input=" + name );
      for ( const auto & id : query.execute( dbRO->db ) )
        {
          thisInputIds.emplace_back( id );
        }
      inputs.emplace_back( input );

      globalResultCount += thisInputIds.size();
      debugLog( "found " + std::to_string( thisInputIds.size() )
                + " results, input=" + name );
      globallyFoundIds.emplace_back( std::move( thisInputIds ) );
    }

  /* Return results as a single flat list by iterating over each input with
   * results */
  debugLog( "found " + std::to_string( globalResultCount )
            + " total results across all inputs" );
  if ( query.limit.has_value() )
    {
      debugLog( "returning the first " + std::to_string( *query.limit )
                + " results" );
      // Emit the number of results as the first line
      nlohmann::json resultCountRecord
        = { { "result-count", globalResultCount } };
      std::cout << resultCountRecord << std::endl;
      // Only print the first `limit` results
      for ( size_t i = 0; i < inputs.size(); i++ )
        {
          if ( *query.limit == 0 ) { break; }
          auto input    = inputs[i];
          auto inputIds = globallyFoundIds[i];
          for ( auto & id : inputIds )
            {
              if ( *query.limit == 0 ) { break; }
              std::cout << input->getRowJSON( id ).dump() << std::endl;
              *query.limit -= 1;
            }
        }
    }
  else
    {
      debugLog( "returning all results" );
      // Print all of the results
      for ( size_t i = 0; i < inputs.size(); i++ )
        {
          auto input    = inputs[i];
          auto inputIds = globallyFoundIds[i];
          for ( auto & id : inputIds )
            {
              std::cout << input->getRowJSON( id ).dump() << std::endl;
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
