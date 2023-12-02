/* ========================================================================== *
 *
 * @file pkgdb/get.cc
 *
 * @brief Implementation of `pkgdb get` subcommand.
 *
 *
 * -------------------------------------------------------------------------- */

#include <iostream>

#include <nlohmann/json.hpp>

#include "flox/pkgdb/command.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

GetCommand::GetCommand()
  : parser( "get" )
  , pId( "id" )
  , pPath( "path" )
  , pDone( "done" )
  , pFlake( "flake" )
  , pDb( "db" )
  , pPkg( "pkg" )
{
  this->parser.add_description( "Get metadata from Package DB" );

  this->pId.add_description( "Lookup an attribute set or package row `id`" );
  this->pId.add_argument( "-p", "--pkg" )
    .help( "lookup package path" )
    .nargs( 0 )
    .action( [&]( const auto & ) { this->isPkg = true; } );
  this->addTargetArg( this->pId );
  this->addAttrPathArgs( this->pId );
  this->parser.add_subparser( this->pId );

  this->pDone.add_description(
    "Check to see if an attrset and its children has been scraped" );
  this->addTargetArg( this->pDone );
  this->addAttrPathArgs( this->pDone );
  this->parser.add_subparser( this->pDone );

  this->pPath.add_description(
    "Lookup an (AttrSets|Packages).id attribute path" );
  this->pPath.add_argument( "-p", "--pkg" )
    .help( "lookup `Packages.id'" )
    .nargs( 0 )
    .action( [&]( const auto & ) { this->isPkg = true; } );
  this->addTargetArg( this->pPath );
  this->pPath.add_argument( "id" )
    .help( "row `id' to lookup" )
    .nargs( 1 )
    .action( [&]( const std::string & rowId )
             { this->id = std::stoull( rowId ); } );
  this->parser.add_subparser( this->pPath );

  this->pFlake.add_description( "Get flake metadata from Package DB" );
  this->addTargetArg( this->pFlake );
  this->parser.add_subparser( this->pFlake );

  this->pDb.add_description( "Get absolute path to Package DB for a flake" );
  this->addTargetArg( this->pDb );
  this->parser.add_subparser( this->pDb );

  this->pPkg.add_description( "Get info about a single package" );
  this->addTargetArg( this->pPkg );
  /* In `runPkg' we check for a singleton and if it's an integer it
   * is interpreted as a row id. */
  this->pPkg.add_argument( "id-or-path" )
    .help( "attribute path to package, or `Packages.id`" )
    .metavar( "<ID|ATTRS...>" )
    .remaining()
    .action( [&]( const std::string & idOrPath )
             { this->attrPath.emplace_back( idOrPath ); } );
  this->parser.add_subparser( this->pPkg );
}


/* -------------------------------------------------------------------------- */

int
GetCommand::runId()
{
  if ( this->isPkg )
    {
      std::cout << this->db->getPackageId( this->attrPath ) << std::endl;
    }
  else { std::cout << this->db->getAttrSetId( this->attrPath ) << std::endl; }
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

int
GetCommand::runDone()
{
  if ( this->db->completedAttrSet( this->attrPath ) )
    {
      if ( nix::lvlNotice < nix::verbosity )
        {
          std::cout << "true" << std::endl;
        }
      return EXIT_SUCCESS;
    }
  if ( nix::lvlNotice < nix::verbosity ) { std::cout << "false" << std::endl; }
  return EXIT_FAILURE;
}


/* -------------------------------------------------------------------------- */

int
GetCommand::runPath()
{
  if ( this->isPkg )
    {
      std::cout << nlohmann::json( this->db->getPackagePath( this->id ) ).dump()
                << std::endl;
    }
  else
    {
      std::cout << nlohmann::json( this->db->getAttrSetPath( this->id ) ).dump()
                << std::endl;
    }
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

int
GetCommand::runFlake()
{
  nlohmann::json flakeInfo
    = { { "string", this->db->lockedRef.string },
        { "attrs", this->db->lockedRef.attrs },
        { "fingerprint",
          this->db->fingerprint.to_string( nix::Base16, false ) } };
  std::cout << flakeInfo.dump() << std::endl;
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

int
GetCommand::runDb()
{
  if ( this->dbPath.has_value() )
    {
      std::cout << static_cast<std::string>( *this->dbPath ) << std::endl;
    }
  else
    {
      std::string dbPath(
        pkgdb::genPkgDbName( this->flake->lockedFlake.getFingerprint() ) );
      std::cout << dbPath << std::endl;
    }
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

int
GetCommand::runPkg()
{
  if ( ( this->attrPath.size() == 1 ) && ( isUInt( this->attrPath.front() ) ) )
    {
      this->id = stoull( this->attrPath.front() );
      this->attrPath.clear();
      std::cout << this->db->getPackage( this->id ) << std::endl;
    }
  else { std::cout << this->db->getPackage( this->attrPath ) << std::endl; }
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

int
GetCommand::run()
{
  if ( this->parser.is_subcommand_used( "db" ) ) { return this->runDb(); }

  this->openPkgDb();

  if ( this->parser.is_subcommand_used( "id" ) ) { return this->runId(); }
  if ( this->parser.is_subcommand_used( "path" ) ) { return this->runPath(); }
  if ( this->parser.is_subcommand_used( "flake" ) ) { return this->runFlake(); }
  if ( this->parser.is_subcommand_used( "done" ) ) { return this->runDone(); }
  if ( this->parser.is_subcommand_used( "pkg" ) ) { return this->runPkg(); }
  std::cerr << this->parser << std::endl;
  throw flox::FloxException( "You must provide a valid `get' subcommand" );
  return EXIT_FAILURE;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
