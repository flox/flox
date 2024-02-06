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
   * we skip straight to trying to create a read-only connection to
   * the database.
   * However, just because the database exists doesn't mean that it's done being
   * initialized, so creating the read-only connection can fail. */
  try
    {
      this->dbRO = std::make_shared<PkgDbReadOnly>(
        this->getFlake()->lockedFlake.getFingerprint(),
        this->dbPath.string() );
    }
  catch ( ... )
    {
      throw PkgDbException( "couldn't initialize read-only package database" );
    }

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

  Todos todo;
  bool  wasRW = this->dbRW != nullptr;

  std::cout << "WML: Scrape prefix " << prefix.back() << std::endl;

  // {
  //   MaybeCursor root = this->getFlake()->maybeOpenCursor( prefix );
  //   if ( root == nullptr ) { return; }
  // }
  
  // close the db if we have anything open in preparation for the child to take over.
  this->closeDbReadWrite();

  // split this->getFlake()->state->symbols into 1k symbol chunks
  // auto symbolTableChunks = split( this->getFlake()->state->symbols, 1000);
  // auto symbolTableChunks
  //   = std::vector<nix::SymbolTable> { this->getFlake()->state->symbols };

  do {

      pid_t pid = fork();
      if ( pid == -1 ) { throw PkgDbException( "fork faild" ); }
      if ( 0 < pid )
        {
          int status = 0;
          std::cout << "WML: scraping in child: " << pid << std::endl;
          waitpid( pid, &status, 0 );
          if ( WEXITSTATUS( status ) != EXIT_SUCCESS )
            {
              throw PkgDbException( nix::fmt( "scraping failed: exit code %d",
                                              WEXITSTATUS( status ) ) );
            }
          std::cout << "WML: child exited: status: " << status << std::endl;
        }
      else
        {
          /* Open a read/write connection. */
          auto   chunkDbRW = this->getDbReadWrite();
          row_id chunkRow  = chunkDbRW->addOrGetAttrSetId( prefix );
          MaybeCursor root = this->getFlake()->maybeOpenCursor( prefix );


          todo.emplace( std::make_tuple( prefix,
                                         static_cast<flox::Cursor>( root ),
                                         chunkRow ) );

          /* Start a transaction */
          chunkDbRW->execute( "BEGIN EXCLUSIVE TRANSACTION" );

          try
            {
              while ( ! todo.empty() )
                {
                  // std::cout << "WML: calling 'scrape' in child...." << std::endl;
                  chunkDbRW->scrape( this->getFlake()->state->symbols,
                                     todo.front(),
                                     todo );
                  todo.pop();
                }
            }
          catch ( const nix::EvalError & err )
            {
              chunkDbRW->execute( "ROLLBACK TRANSACTION" );
              /* Close the r/w connection if we opened it. */
              if ( ! wasRW ) { this->closeDbReadWrite(); }
              throw NixEvalException( "error scraping flake", err );
            }

          /* Close the transaction. */
          chunkDbRW->execute( "COMMIT TRANSACTION" );
          std::cout << "WML: scaping complete in child...." << std::endl;
          exit(0);
        }
    }
  while ( false );

  /* Open a read/write connection. */
  auto        dbRW = this->getDbReadWrite();
  row_id      row  = dbRW->addOrGetAttrSetId( prefix );

  /* Mark the prefix and its descendants as "done" */
  std::cout << "WML: marking prefix row " << row << " complete " << std::endl;
  dbRW->execute( "BEGIN TRANSACTION" );
  dbRW->setPrefixDone( row, true );
  dbRW->execute( "COMMIT TRANSACTION" );

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
      std::cout << "WML: scraping systems for input." << std::endl;
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
