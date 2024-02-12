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

  // close the db if we have anything open in preparation for the child to take
  // over.
  this->closeDbReadWrite();
  this->freeFlake();

  bool         scrapingComplete = false;
  const size_t pageSize         = 5000;
  size_t       pageIdx          = 0;

  do {
      const int EXIT_CHILD_INCOMPLETE = EXIT_SUCCESS + 1;
      const int EXIT_FAILURE_NIX_EVAL
        = 150;  // seems to not overlap with common posix codes

      pid_t pid = fork();
      if ( pid == -1 )
        {
          throw PkgDbException( "fork to scrape attributes failed" );
        }
      if ( 0 < pid )
        {
          int status = 0;
          infoLog( nix::fmt( "scrapePrefix: Waiting for forked process, pid:%d",
                             pid ) );
          waitpid( pid, &status, 0 );
          infoLog( nix::fmt( "scrapePrefix: Forked process exited, exitcode:%d",
                             status ) );

          if ( WIFEXITED( status ) )
            {
              if ( WEXITSTATUS( status ) == EXIT_SUCCESS )
                {
                  infoLog( nix::fmt(
                    "scrapePrefix: Child reports all pages complete" ) );
                  scrapingComplete = true;
                }
              else if ( WEXITSTATUS( status ) == EXIT_CHILD_INCOMPLETE )
                {
                  infoLog( nix::fmt( "scrapePrefix: Child reports additional "
                                     "pages to process" ) );
                  // Make sure to increment the pageIdx here (in the parent)
                  pageIdx++;
                  scrapingComplete = false;
                }
              else  // ( WEXITSTATUS( status ) != EXIT_SUCCESS )
                {
                  scrapingComplete = true;
                  infoLog( nix::fmt(
                    "scrapePrefix: Child reports failure, aborting" ) );
                  if ( WEXITSTATUS( status ) == EXIT_FAILURE_NIX_EVAL )
                    {
                      throw PkgDbException(
                        nix::fmt( "scraping failed: NixEvalException reported. "
                                  "See child log for details." ) );
                    }
                  else
                    {
                      throw PkgDbException(
                        nix::fmt( "scraping failed: exit code %d",
                                  WEXITSTATUS( status ) ) );
                    }
                }
            }
          else
            {
              scrapingComplete = true;
              if ( WTERMSIG( status ) != SIGTERM )
                {
                  throw PkgDbException(
                    nix::fmt( "scraping failed: abnormal child exit, signal:%d",
                              WTERMSIG( status ) ) );
                }
            }
        }
      else
        {
          /* Open a read/write connection. */
          auto chunkDbRW = this->getDbReadWrite();

          /* Start a transaction */
          chunkDbRW->execute( "BEGIN TRANSACTION" );
          row_id      chunkRow = chunkDbRW->addOrGetAttrSetId( prefix );
          MaybeCursor root     = this->getFlake()->maybeOpenCursor( prefix );

          Target rootTarget
            = std::make_tuple( prefix,
                               static_cast<flox::Cursor>( root ),
                               chunkRow );
          bool targetComplete = false;

          try
            {
              infoLog(
                nix::fmt( "scrapePrefix(child): scraping page %d of %d attribs",
                          pageIdx,
                          pageSize ) );
              targetComplete
                = chunkDbRW->scrape( this->getFlake()->state->symbols,
                                     rootTarget,
                                     pageSize,
                                     pageIdx );
            }
          catch ( const nix::EvalError & err )
            {
              infoLog(
                nix::fmt( "scrapePrefix(child): caught nix::EvalError: %s",
                          err.msg().c_str() ) );
              chunkDbRW->execute( "ROLLBACK TRANSACTION" );
              infoLog( nix::fmt(
                "scrapePrefix(child): eval error, closing db and flake" ) );
              this->closeDbReadWrite();
              this->freeFlake();
              exit( EXIT_FAILURE_NIX_EVAL );
            }

          infoLog( nix::fmt(
            "scrapePrefix(child): done scraping, commiting transaction" ) );

          /* Close the transaction. */
          chunkDbRW->execute( "COMMIT TRANSACTION" );
          infoLog(
            nix::fmt( "scrapePrefix(child): done scraping, closing db" ) );
          this->closeDbReadWrite();
          infoLog(
            nix::fmt( "scrapePrefix(child): done scraping, freeing flake" ) );
          this->freeFlake();

          infoLog( nix::fmt(
            "scrapePrefix(child): scraping page %d complete, lastPage:%d",
            pageIdx,
            targetComplete ) );
          raise( SIGTERM );
          exit( targetComplete ? EXIT_SUCCESS : EXIT_CHILD_INCOMPLETE );
        }
    }
  while ( ! scrapingComplete );
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
