/* ========================================================================== *
 *
 * @file pkgdb/input.cc
 *
 * @brief Helpers for managing package database inputs and state.
 *
 *
 * -------------------------------------------------------------------------- */

#include <cassert>
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
#include "flox/pkgdb/scrape-rules.hh"
#include "flox/pkgdb/write.hh"


/* -------------------------------------------------------------------------- */

/* Forward declare */
namespace nix {
class Store;
}


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

bool
PkgDbInput::initDbRO()
{
  bool isFresh = false;

  /**
   * Initialize DB if missing.
   *
   * Databases are initialized as a temporary file,
   * then hard linked to the final location.
   * The hard link is atomic, and the temporary file is removed.
   *
   * This way, we should be able to prevent other processes opening the
   * partially initialized database.
   */
  if ( ! std::filesystem::exists( this->dbPath ) )
    {
      std::filesystem::create_directories( this->dbPath.parent_path() );
      nix::logger->log(
        nix::lvlTalkative,
        nix::fmt( "Creating database '%s'", this->dbPath.string() ) );

      // random 8 char suffix
      std::string tempSuffix = "";
      for ( int i = 0; i < 8; i++ ) { tempSuffix += 'a' + ( rand() % 25 ); }

      auto tempDbPath = std::filesystem::path( this->dbPath )
                          .replace_extension( this->dbPath.extension().string()
                                              + "." + tempSuffix );

      debugLog(
        nix::fmt( "Creating temporary database '%s'", tempDbPath.string() ) );

      PkgDb( this->getFlake()->lockedFlake, tempDbPath.string() );

      try
        {

          debugLog( nix::fmt( "Moving intialized database '%s' -> %s",
                              tempDbPath.string(),
                              this->dbPath.string() ) );
          std::filesystem::create_hard_link( tempDbPath, this->dbPath );
        }
      catch ( const std::exception & e )
        {
          debugLog(
            "Failed to create link, db file created by other process?" );
        }

      std::filesystem::remove( tempDbPath );

      isFresh = true;
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
  catch ( const std::exception & e )
    {
      throw PkgDbException( "couldn't initialize read-only package database",
                            e.what() );
    }

  return isFresh;
}

void
PkgDbInput::init()
{
  /* If this is a fresh Db, we don't need to do any of this checking. */
  if ( ! initDbRO() )
    {
      /* If the schema version is not as expected, or the rules hash is
       * different (rules update), delete the file, free the `dbRo` object in
       * memory, and re-init the file. */
      const ScrapeRules & scrapeRules  = getDefaultRules();
      SqlVersions         dbVersions   = this->dbRO->getDbVersion();
      ScrapeMeta          dbScrapeMeta = this->dbRO->getDbScrapeMeta();
      if ( bool rulesMatch
           = ( dbScrapeMeta.rulesHash == scrapeRules.hashString() );
           dbVersions.tables != sqlVersions.tables || ! rulesMatch )
        {
          nix::logger->log(
            nix::lvlTalkative,
            nix::fmt( "Outdated database '%s'", this->dbPath.string() ) );
          nix::logger->log( nix::lvlTalkative,
                            nix::fmt( "Clearing due to %s",
                                      rulesMatch
                                        ? "table schema being outdated"
                                        : "scraping rules mismatch" ) );
          /* Delete the file, free the in memory Db, and re-create it. */
          this->dbRO = nullptr;
          std::filesystem::remove( this->dbPath );
          initDbRO();
        }
      else if ( dbVersions.views != sqlVersions.views )
        {
          /* This will actually do much more than updating the views, but it is
           * handled correctly in SQL. */
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


int
PkgDbInput::getScrapingPageSize()
{
  // Each entry (in order) is checked if the avaialble memory is >= memKb, and
  // if so, will use pageSize.
  struct MemThreshold
  {
    long   memoryKb;
    size_t pageSize;
  };
  // These are very rough heuristics.  It was found that about 4.5g is required
  // to scrape the entire darwin subtree all at once.  1000 item page sizes
  // seems to keep memory consumption under 1.5g.  These values are a
  // conservative estimate with the hopes of never OOMing.  That said, the
  // method of determining *available* memory is to count reported free memory,
  // and also including *shared* and *cache/buffer* allocated memory thinking
  // that it could be re-allocated.  The amount of truly *free* memory (at least
  // on linux) is usually relatively low.
  const std::vector<MemThreshold> MemThresholds = {
    { 6 /* Gb */ * ( 1024 * 1024L ), PkgDbInput::maxPageSize },
    { 4 /* Gb */ * ( 1024 * 1024L ), 20 * 1000UL },
    { 3 /* Gb */ * ( 1024 * 1024L ), 10 * 1000UL },
    { 2 /* Gb */ * ( 1024 * 1024L ), 4 * 1000UL },
  };

  // No override, so use heuristics
  long availableMemory = getAvailableSystemMemory();

  debugLog( nix::fmt( "getScrapingPageSize: using available memory as: %dkb",
                      availableMemory ) );

  for ( auto threshold : MemThresholds )
    {
      traceLog( nix::fmt( "getScrapingPageSize: checking threshold: %dkb",
                          threshold.memoryKb ) );
      if ( availableMemory > threshold.memoryKb )
        {
          debugLog( nix::fmt( "getScrapingPageSize: using page size: %d",
                              threshold.pageSize ) );
          return threshold.pageSize;
        }
    }

  // Use the minimum and warn in the output
  verboseLog( "getScrapingPageSize: using minimum page size, performance will "
              "be impacted!" );
  return PkgDbInput::minPageSize;
}

// NOLINTBEGIN cognitive complexity (nesting and logging macros)
void
PkgDbInput::scrapePrefix( const flox::AttrPath & prefix )
{
  if ( this->getDbReadOnly()->completedAttrSet( prefix ) ) { return; }

  Todos todo;

  // Close the db and clean up if we have anything open in preparation for the
  // child to take over.
  this->closeDbReadWrite();
  this->freeFlake();

  bool         scrapingComplete = false;
  const size_t pageSize         = getScrapingPageSize();
  size_t       pageIdx          = 0;

  while ( ! scrapingComplete )
    {
      pid_t pid = fork();
      if ( pid == -1 )
        {
          throw PkgDbException( "fork to scrape attributes failed" );
        }
      if ( 0 < pid )
        {
          //
          // This is the parent process
          int status = 0;
          debugLog(
            nix::fmt( "scrapePrefix: Waiting for forked process, pid: %d",
                      pid ) );
          waitpid( pid, &status, 0 );
          debugLog(
            nix::fmt( "scrapePrefix: Forked process exited, exitcode: %d",
                      status ) );

          if ( WIFEXITED( status ) )
            {
              if ( WEXITSTATUS( status ) == EXIT_SUCCESS )
                {
                  debugLog( "scrapePrefix: Child reports all pages complete" );
                  scrapingComplete = true;
                }
              else if ( WEXITSTATUS( status ) == EXIT_CHILD_INCOMPLETE )
                {
                  debugLog( "scrapePrefix: Child reports additional "
                            "pages to process" );
                  // Make sure to increment the pageIdx here (in the parent)
                  pageIdx++;
                  scrapingComplete = false;
                }
              else  // ( WEXITSTATUS( status ) != EXIT_SUCCESS )
                {
                  debugLog( "scrapePrefix: Child reports failure, aborting" );
                  if ( WEXITSTATUS( status ) == EXIT_FAILURE_NIX_EVAL )
                    {
                      throw PkgDbException(
                        "scraping failed: NixEvalException reported. "
                        "See child log for details." );
                    }

                  throw PkgDbException(
                    nix::fmt( "scraping failed: exit code %d",
                              WEXITSTATUS( status ) ) );
                }
            }
          else
            {
              throw PkgDbException(
                nix::fmt( "scraping failed: abnormal child exit, signal: %d",
                          WTERMSIG( status ) ) );
            }
        }
      else
        {
          /*
           * It is critical for the forked child process to NOT run the exit
           * handlers (as will be done in calling `exit()`).
           * Doing so will cause the child to try and cleanup threads and such,
           * that the parent is still using, specifically the nix download
           * thread. Calling `_exit()` does not call the exit handlers and
           * allows the child to exit cleanly without interrupting the parent.
           */
          _exit( scrapePrefixWorker( this, prefix, pageIdx, pageSize ) );
        }
    }
}
// NOLINTEND

int
PkgDbInput::scrapePrefixWorker( PkgDbInput *     input,
                                const AttrPath & prefix,
                                const size_t     pageIdx,
                                const size_t     pageSize )
{
  /* Open a read/write connection. */
  auto chunkDbRW = input->getDbReadWrite();

  /* Start a transaction */
  chunkDbRW->execute( "BEGIN TRANSACTION" );
  row_id      chunkRow = chunkDbRW->addOrGetAttrSetId( prefix );
  MaybeCursor root     = input->getFlake()->maybeOpenCursor( prefix );

  Target rootTarget
    = std::make_tuple( prefix, static_cast<flox::Cursor>( root ), chunkRow );
  bool targetComplete = false;

  try
    {
      debugLog( nix::fmt( "scrapePrefix(child): scraping page %d of "
                          "%d attributes",
                          pageIdx,
                          pageSize ) );
      targetComplete = chunkDbRW->scrape( input->getFlake()->state->symbols,
                                          rootTarget,
                                          pageSize,
                                          pageIdx );
    }
  catch ( const nix::EvalError & err )
    {
      debugLog( nix::fmt( "scrapePrefix(child): caught nix::EvalError: %s",
                          err.msg().c_str() ) );
      chunkDbRW->execute( "ROLLBACK TRANSACTION" );
      input->closeDbReadWrite();
      input->freeFlake();
      return EXIT_FAILURE_NIX_EVAL;
    }

  /* Close the transaction. */
  chunkDbRW->execute( "COMMIT TRANSACTION" );
  debugLog(
    nix::fmt( "scrapePrefix(child): scraping page %d complete, lastPage: %d",
              pageIdx,
              targetComplete ) );
  try
    {
      input->closeDbReadWrite();
      input->freeFlake();
      return targetComplete ? EXIT_SUCCESS : EXIT_CHILD_INCOMPLETE;
    }
  catch ( const std::exception & err )
    {
      debugLog( nix::fmt( "scrapePrefix(child): caught exception on exit: %s",
                          err.what() ) );
      return targetComplete ? EXIT_SUCCESS : EXIT_CHILD_INCOMPLETE;
    }
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
