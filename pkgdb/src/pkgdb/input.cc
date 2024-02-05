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
#include <sys/wait.h>
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
          std::this_thread::sleep_for( DB_RETRY_PERIOD );
          if ( DB_MAX_RETRIES < ++retries )
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
  std::shared_ptr<PkgDb> dbRW = this->getDbReadWrite();
  row_id                 row  = dbRW->addOrGetAttrSetId( prefix );

  todo.emplace(
    std::make_tuple( prefix, static_cast<flox::Cursor>( root ), row ) );

  /* Start a transaction */
  while ( ! todo.empty() )
    {
      dbRW->execute( "BEGIN TRANSACTION" );
      try
        {
          dbRW->scrape( this->getFlake()->state->symbols, todo.front(), todo );
          todo.pop();
        }
      catch ( const nix::EvalError & err )
        {
          dbRW->execute( "ROLLBACK TRANSACTION" );
          /* Close the r/w connection if we opened it. */
          if ( ! wasRW ) { this->closeDbReadWrite(); }
          throw NixEvalException( "error scraping flake", err );
        }
      catch ( const std::bad_alloc & )
        {
          /* Commit and close so a sibling can complete with our progress. */
          dbRW->execute( "COMMIT TRANSACTION" );
          /* Reopen the r/w connection if we opened it. */
          if ( ! wasRW )
            {
              dbRW = nullptr;
              this->closeDbReadWrite();
              dbRW = this->getDbReadWrite();
            }
        }
    }

  /* Mark the prefix and its descendants as "done" */
  dbRW->setPrefixDone( row, true );

  /* Close the transaction. */
  dbRW->execute( "COMMIT TRANSACTION" );

  /* Close the r/w connection if we opened it. */
  if ( ! wasRW ) { this->closeDbReadWrite(); }
}


/* -------------------------------------------------------------------------- */

constexpr int CHILD_NOMEM_STATUS = EXIT_FAILURE + 1;


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
          for ( size_t retries = 0; retries < MAX_SCRAPES; ++retries )
            {
              pid_t pid = fork();
              if ( pid == -1 ) { throw PkgDbException( "fork failed" ); }
              if ( 0 < pid )
                {
                  int status;
                  waitpid( pid, &status, 0 );
                  if ( WIFEXITED( status ) )
                    {
                      if ( WEXITSTATUS( status ) == CHILD_NOMEM_STATUS )
                        {
                          // TODO: make a debug message
                          // if ( nix::lvlDebug <= nix::verbosity )
                          if ( nix::lvlInfo <= nix::verbosity )
                            {
                              // debugLog(
                              infoLog(
                                nix::fmt( "OOM while scraping '%s.%s'. "
                                          "Continuing in sibling process.",
                                          concatStringsSep( ".", prefix ),
                                          system ) );
                            }
                          continue;
                        }
                      if ( WEXITSTATUS( status ) != EXIT_SUCCESS )
                        {
                          throw PkgDbException(
                            nix::fmt( "scraping child failed: exit code %d",
                                      WEXITSTATUS( status ) ) );
                        }
                      break;
                    }
                }
              else
                {
                  prefix.emplace_back( system );
                  try
                    {
                      this->scrapePrefix( prefix );
                    }
                  catch ( const std::bad_alloc & )
                    {
                      exit( CHILD_NOMEM_STATUS );
                    }
                  catch ( const std::exception & e )
                    {
                      if ( nix::lvlError <= nix::verbosity )
                        {
                          errorLog( nix::fmt( "scraping '%s' failed: %s",
                                              concatStringsSep( ".", prefix ),
                                              e.what() ) );
                        }
                      throw;
                    }
                  catch ( ... )
                    {
                      throw;
                    }
                  prefix.pop_back();
                  exit( EXIT_SUCCESS );
                }
            }
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
