/* ========================================================================== *
 *
 * @file pkgdb/input.cc
 *
 * @brief Helpers for managing package database inputs and state.
 *
 *
 * -------------------------------------------------------------------------- */

#include <assert.h>
#include <list>
#include <map>
#include <optional>
#include <ostream>
#include <tuple>

#include <nix/error.hh>
#include <nix/eval.hh>
#include <nix/fmt.hh>
#include <nix/logging.hh>
#include <nix/nixexpr.hh>
#include <nlohmann/json.hpp>
#include <sqlite3pp.hh>

#include "flox/core/exceptions.hh"
#include "flox/pkgdb/input.hh"
#include "flox/pkgdb/write.hh"


/* -------------------------------------------------------------------------- */

/* Forward declare */
namespace nix {
class Store;
}


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

void
PkgDbInput::init()
{
  /* Initialize DB if missing. */
  if ( ! std::filesystem::exists( this->dbPath ) )
    {
      std::filesystem::create_directories( this->dbPath.parent_path() );
      nix::logger->log(
        nix::lvlTalkative,
        nix::fmt( "Creating database '%s'", this->dbPath.string() ) );
      PkgDb( this->getFlake()->lockedFlake, this->dbPath.string() );
    }

  /* If the database exists we don't want to needlessly try to initialize it, so
  we skip straight to trying to create a read-only connection to the database.
  However, just because the database exists doesn't mean that it's done being
  initialized, so creating the read-only connection can fail. We do this retry
  loop to until creating the read-only connection succeeds. */
  /* TODO: emit the number of retries? */
  int retries = 0;
  do {
      try
        {
          this->dbRO = std::make_shared<PkgDbReadOnly>(
            this->getFlake()->lockedFlake.getFingerprint(),
            this->dbPath.string() );
        }
      catch ( ... )
        {
          std::this_thread::sleep_for( DurationMillis( 250 ) );
          if ( ++retries > 100 )
            {
              throw PkgDbException(
                "couldn't initialize read-only package database" );
            }
        }
    }
  while ( ( this->dbRO == nullptr ) );

  /* If the schema version is bad, delete the DB so it will be recreated. */
  SqlVersions dbVersions = this->dbRO->getDbVersion();
  if ( dbVersions.tables != sqlVersions.tables )
    {
      nix::logger->log(
        nix::lvlTalkative,
        nix::fmt( "Clearing outdated database '%s'", this->dbPath.string() ) );
      std::filesystem::remove( this->dbPath );
      PkgDb( this->getFlake()->lockedFlake, this->dbPath.string() );
    }
  else if ( dbVersions.views != sqlVersions.views )
    {
      nix::logger->log( nix::lvlTalkative,
                        nix::fmt( "Updating outdated database views '%s'",
                                  this->dbPath.string() ) );
      PkgDb( this->getFlake()->lockedFlake, this->dbPath.string() );
    }

  /* If the schema version is still wrong throw an error, but we don't
   * expect this to actually occur. */
  dbVersions = this->dbRO->getDbVersion();
  if ( dbVersions != sqlVersions )
    {
      throw PkgDbException(
        nix::fmt( "Incompatible Flox PkgDb schema versions ( %u, %u )",
                  dbVersions.tables,
                  dbVersions.views ) );
    }
}


/* -------------------------------------------------------------------------- */

nix::ref<PkgDb>
PkgDbInput::getDbReadWrite()
{
  if ( this->dbRW == nullptr )
    {
      this->dbRW = std::make_shared<PkgDb>( this->getFlake()->lockedFlake,
                                            this->dbPath.string() );
    }
  return static_cast<nix::ref<PkgDb>>( this->dbRW );
}


/* -------------------------------------------------------------------------- */

void
PkgDbInput::closeDbReadWrite()
{
  if ( this->dbRW != nullptr ) { this->dbRW = nullptr; }
}


/* -------------------------------------------------------------------------- */

void
PkgDbInput::scrapePrefix( const flox::AttrPath & prefix )
{
  if ( this->getDbReadOnly()->completedAttrSet( prefix ) ) { return; }

  Todos       todo;
  bool        wasRW = this->dbRW != nullptr;
  MaybeCursor root  = this->getFlake()->maybeOpenCursor( prefix );

  if ( root == nullptr ) { return; }

  /* Open a read/write connection. */
  auto   dbRW = this->getDbReadWrite();
  row_id row  = dbRW->addOrGetAttrSetId( prefix );

  todo.emplace(
    std::make_tuple( prefix, static_cast<flox::Cursor>( root ), row ) );

  /* Start a transaction */
  dbRW->db.execute( "BEGIN EXCLUSIVE TRANSACTION" );
  try
    {
      while ( ! todo.empty() )
        {
          dbRW->scrape( this->getFlake()->state->symbols, todo.front(), todo );
          todo.pop();
        }

      /* Mark the prefix and its descendants as "done" */
      dbRW->setPrefixDone( row, true );
    }
  catch ( const nix::EvalError & err )
    {
      dbRW->db.execute( "ROLLBACK TRANSACTION" );
      /* Close the r/w connection if we opened it. */
      if ( ! wasRW ) { this->closeDbReadWrite(); }
      throw NixEvalException( "error scraping flake", err );
    }

  /* Close the transaction. */
  dbRW->db.execute( "COMMIT TRANSACTION" );

  /* Close the r/w connection if we opened it. */
  if ( ! wasRW ) { this->closeDbReadWrite(); }
}


/* -------------------------------------------------------------------------- */

void
PkgDbInput::scrapeSystems( const std::vector<System> & systems )
{
  /* Loop and scrape over `subtrees' and `systems'. */
  for ( const auto & subtree : this->getSubtrees() )
    {
      flox::AttrPath prefix
        = { static_cast<std::string>( to_string( subtree ) ) };
      for ( const auto & system : systems )
        {
          prefix.emplace_back( system );
          this->scrapePrefix( prefix );
          prefix.pop_back();
        }
    }
}


/* -------------------------------------------------------------------------- */

nlohmann::json
PkgDbInput::getRowJSON( row_id row )
{
  auto dbRO = this->getDbReadOnly();
  auto rsl  = dbRO->getPackage( row );
  rsl.emplace( "input", this->getNameOrURL() );
  return rsl;
}


/* -------------------------------------------------------------------------- */

void
PkgDbRegistryMixin::initRegistry()
{
  if ( this->registry == nullptr )
    {
      nix::ref<nix::Store>     store = this->getStore();
      pkgdb::PkgDbInputFactory factory( store );  // TODO: cacheDir
      this->registry
        = std::make_shared<Registry<PkgDbInputFactory>>( this->getRegistryRaw(),
                                                         factory );
    }
}


/* -------------------------------------------------------------------------- */

void
PkgDbRegistryMixin::scrapeIfNeeded()
{
  this->initRegistry();
  assert( this->registry != nullptr );
  for ( auto & [name, input] : *this->registry )
    {
      input->scrapeSystems( this->getSystems() );
    }
}


/* -------------------------------------------------------------------------- */

nix::ref<Registry<PkgDbInputFactory>>
PkgDbRegistryMixin::getPkgDbRegistry()
{
  if ( this->registry == nullptr ) { this->scrapeIfNeeded(); }
  assert( this->registry != nullptr );
  return static_cast<nix::ref<Registry<PkgDbInputFactory>>>( this->registry );
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
