/* ========================================================================== *
 *
 * @file pkgdb/read.cc
 *
 * @brief Implementations for reading a SQLite3 package set database.
 *
 *
 * -------------------------------------------------------------------------- */

#include <chrono>
#include <fstream>
#include <functional>
#include <limits>
#include <list>
#include <memory>
#include <string>
#include <thread>
#include <unordered_set>
#include <vector>

#include "flox/flake-package.hh"
#include "flox/pkgdb/read.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

std::atomic_flag shouldStopFlag = ATOMIC_FLAG_INIT;

/* -------------------------------------------------------------------------- */

bool
dbIsBusy( int rcode )
{
  traceLog( "sqlite return code was " + std::to_string( rcode ) );
  return ( rcode == SQLITE_BUSY ) || ( rcode == SQLITE_BUSY_SNAPSHOT );
}

int
retryWhileBusy( sqlite3pp::command & cmd )
{
  using namespace std::chrono_literals;
  int rcode = cmd.execute();
  while ( dbIsBusy( rcode ) )
    {
      std::this_thread::sleep_for( 500ms );
      rcode = cmd.execute();
    }
  return rcode;
}

int
retryAllWhileBusy( sqlite3pp::command & cmd )
{
  using namespace std::chrono_literals;
  int rcode = cmd.execute_all();
  while ( dbIsBusy( rcode ) )
    {
      std::this_thread::sleep_for( 500ms );
      rcode = cmd.execute_all();
    }
  return rcode;
}


/* -------------------------------------------------------------------------- */

std::filesystem::path
getPkgDbCachedir()
{
  /* Generate a dirname from the SQL tables version number. Only run once. */
  static bool              known = false;
  static std::stringstream dirname;
  if ( ! known )
    {
      dirname << nix::getCacheDir() << "/flox/pkgdb-v" << sqlVersions.tables;
      known = true;
    }
  static const std::filesystem::path cacheDir = dirname.str();

  std::optional<std::string> fromEnv = nix::getEnv( "PKGDB_CACHEDIR" );

  if ( fromEnv.has_value() ) { return *fromEnv; }

  return cacheDir;
}


/* -------------------------------------------------------------------------- */

std::filesystem::path
genPkgDbName( const Fingerprint &           fingerprint,
              const std::filesystem::path & cacheDir )
{
  std::string fpStr = fingerprint.to_string( nix::Base16, false );
  return cacheDir.string() + "/" + fpStr + ".sqlite";
}

/* -------------------------------------------------------------------------- */

std::ostream &
operator<<( std::ostream & oss, const SqlVersions & versions )
{
  return oss << "tables: " << versions.tables << ", views: " << versions.views;
}


/* -------------------------------------------------------------------------- */

void
PkgDbReadOnly::init()
{
  if ( ! std::filesystem::exists( this->dbPath ) )
    {
      throw NoSuchDatabase( *this );
    }
  this->db.connect( this->dbPath.string().c_str(), SQLITE_OPEN_READONLY );
  this->loadLockedFlake();
}


/* -------------------------------------------------------------------------- */

void
PkgDbReadOnly::loadLockedFlake()
{
  sqlite3pp::query qry(
    this->db,
    "SELECT fingerprint, string, attrs FROM LockedFlake LIMIT 1" );
  auto      rsl            = *qry.begin();
  auto      fingerprintStr = rsl.get<std::string>( 0 );
  nix::Hash fingerprint
    = nix::Hash::parseNonSRIUnprefixed( fingerprintStr, nix::htSHA256 );
  this->lockedRef.string = rsl.get<std::string>( 1 );
  this->lockedRef.attrs  = nlohmann::json::parse( rsl.get<std::string>( 2 ) );
  /* Check to see if our fingerprint is already known.
   * If it isn't load it, otherwise assert it matches. */
  if ( this->fingerprint == nix::Hash( nix::htSHA256 ) )
    {
      this->fingerprint = fingerprint;
    }
  else if ( this->fingerprint != fingerprint )
    {
      throw PkgDbException(
        nix::fmt( "database '%s' fingerprint '%s' does not match expected '%s'",
                  this->dbPath,
                  fingerprintStr,
                  this->fingerprint.to_string( nix::Base16, false ) ) );
    }
}


/* -------------------------------------------------------------------------- */

SqlVersions
PkgDbReadOnly::getDbVersion()
{
  sqlite3pp::query qry(
    this->db,
    "SELECT version FROM DbVersions "
    "WHERE name IN ( 'pkgdb_tables_schema', 'pkgdb_views_schema' ) LIMIT 2" );
  auto   itr    = qry.begin();
  auto   tables = ( *itr ).get<std::string>( 0 );
  auto   views  = ( *++itr ).get<std::string>( 0 );
  char * end    = nullptr;

  static const int base = 10;

  return SqlVersions {
    .tables
    = static_cast<unsigned>( std::strtoul( tables.c_str(), &end, base ) ),
    .views = static_cast<unsigned>( std::strtoul( views.c_str(), &end, base ) )
  };
}


/* -------------------------------------------------------------------------- */

bool
PkgDbReadOnly::completedAttrSet( row_id row )
{
  /* Lookup the `AttrName.id' ( if one exists ) */
  sqlite3pp::query qryId( this->db, "SELECT done FROM AttrSets WHERE id = ?" );
  qryId.bind( 1, static_cast<long long>( row ) );
  auto itr = qryId.begin();
  return ( itr != qryId.end() ) && ( *itr ).get<bool>( 0 );
}


/* -------------------------------------------------------------------------- */

bool
PkgDbReadOnly::completedAttrSet( const flox::AttrPath & path )
{
  /* Lookup the `AttrName.id' ( if one exists ) */
  row_id row = 0;
  for ( const auto & part : path )
    {
      sqlite3pp::query qryId( this->db,
                              "SELECT id, done FROM AttrSets "
                              "WHERE ( attrName = ? ) AND ( parent = ? )" );
      qryId.bind( 1, part, sqlite3pp::copy );
      qryId.bind( 2, static_cast<long long>( row ) );
      auto itr = qryId.begin();
      if ( itr == qryId.end() ) { return false; } /* No such path. */
      /* If a parent attrset is marked `done', then all of it's children
       * are also considered done. */
      if ( ( *itr ).get<bool>( 1 ) ) { return true; }
      row = ( *itr ).get<long long>( 0 );
    }
  return false;
}


/* -------------------------------------------------------------------------- */

bool
PkgDbReadOnly::hasAttrSet( const flox::AttrPath & path )
{
  /* Lookup the `AttrName.id' ( if one exists ) */
  row_id row = 0;
  for ( const auto & part : path )
    {
      sqlite3pp::query qryId(
        this->db,
        "SELECT id FROM AttrSets WHERE ( attrName = ? ) AND ( parent = ? )" );
      qryId.bind( 1, part, sqlite3pp::copy );
      qryId.bind( 2, static_cast<long long>( row ) );
      auto itr = qryId.begin();
      if ( itr == qryId.end() ) { return false; } /* No such path. */
      row = ( *itr ).get<long long>( 0 );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

std::string
PkgDbReadOnly::getDescription( row_id descriptionId )
{
  if ( descriptionId == 0 ) { return ""; }
  /* Lookup the `Description.id' ( if one exists ) */
  sqlite3pp::query qryId( this->db,
                          "SELECT description FROM Descriptions WHERE id = ?" );
  qryId.bind( 1, static_cast<long long>( descriptionId ) );
  auto itr = qryId.begin();
  /* Handle no such path. */
  if ( itr == qryId.end() )
    {
      throw PkgDbException(
        nix::fmt( "No such Descriptions.id %llu.", descriptionId ) );
    }
  return ( *itr ).get<std::string>( 0 );
}


/* -------------------------------------------------------------------------- */

bool
PkgDbReadOnly::hasPackage( const flox::AttrPath & path )
{
  flox::AttrPath parent;
  for ( size_t idx = 0; idx < ( path.size() - 1 ); ++idx )
    {
      parent.emplace_back( path[idx] );
    }

  /* Make sure there are actually packages in the set. */
  row_id           row = this->getAttrSetId( parent );
  sqlite3pp::query qryPkgs( this->db,
                            "SELECT id FROM Packages WHERE ( parentId = ? ) "
                            "AND ( attrName = ? ) LIMIT 1" );
  qryPkgs.bind( 1, static_cast<long long>( row ) );
  qryPkgs.bind( 2, std::string( path.back() ), sqlite3pp::copy );
  return ( *qryPkgs.begin() ).get<int>( 0 ) != 0;
}


/* -------------------------------------------------------------------------- */

row_id
PkgDbReadOnly::getAttrSetId( const flox::AttrPath & path )
{
  /* Lookup the `AttrName.id' ( if one exists ) */
  row_id row = 0;
  for ( const auto & part : path )
    {
      sqlite3pp::query qryId(
        this->db,
        "SELECT id FROM AttrSets "
        "WHERE ( attrName = ? ) AND ( parent = ? ) LIMIT 1" );
      qryId.bind( 1, part, sqlite3pp::copy );
      qryId.bind( 2, static_cast<long long>( row ) );
      auto itr = qryId.begin();
      /* Handle no such path. */
      if ( itr == qryId.end() )
        {
          throw PkgDbException(
            nix::fmt( "No such AttrSet '%s'.",
                      nix::concatStringsSep( ".", path ) ) );
        }
      row = ( *itr ).get<long long>( 0 );
    }

  return row;
}


/* -------------------------------------------------------------------------- */

flox::AttrPath
PkgDbReadOnly::getAttrSetPath( row_id row )
{
  if ( row == 0 ) { return {}; }
  std::list<std::string> path;
  while ( row != 0 )
    {
      sqlite3pp::query qry(
        this->db,
        "SELECT parent, attrName FROM AttrSets WHERE ( id = ? )" );
      qry.bind( 1, static_cast<long long>( row ) );
      auto itr = qry.begin();
      /* Handle no such path. */
      if ( itr == qry.end() )
        {
          throw PkgDbException( nix::fmt( "No such `AttrSet.id' %llu.", row ) );
        }
      row = ( *itr ).get<long long>( 0 );
      path.push_front( ( *itr ).get<std::string>( 1 ) );
    }
  return flox::AttrPath { std::make_move_iterator( std::begin( path ) ),
                          std::make_move_iterator( std::end( path ) ) };
}


/* -------------------------------------------------------------------------- */

row_id
PkgDbReadOnly::getPackageId( const flox::AttrPath & path )
{
  /* Lookup the `AttrName.id' of parent ( if one exists ) */
  flox::AttrPath parentPath = path;
  parentPath.pop_back();

  row_id parent = this->getAttrSetId( parentPath );

  sqlite3pp::query qry(
    this->db,
    "SELECT id FROM Packages WHERE ( parentId = ? ) AND ( attrName = ? )" );
  qry.bind( 1, static_cast<long long>( parent ) );
  qry.bind( 2, path.back(), sqlite3pp::copy );
  auto itr = qry.begin();
  /* Handle no such path. */
  if ( itr == qry.end() )
    {
      throw PkgDbException(
        nix::fmt( "No such package %s.", nix::concatStringsSep( ".", path ) ) );
    }
  return ( *itr ).get<long long>( 0 );
}


/* -------------------------------------------------------------------------- */

flox::AttrPath
PkgDbReadOnly::getPackagePath( row_id row )
{
  if ( row == 0 ) { return {}; }
  sqlite3pp::query qry(
    this->db,
    "SELECT parentId, attrName FROM Packages WHERE ( id = ? )" );
  qry.bind( 1, static_cast<long long>( row ) );
  auto itr = qry.begin();
  /* Handle no such path. */
  if ( itr == qry.end() )
    {
      throw PkgDbException( nix::fmt( "No such `Packages.id' %llu.", row ) );
    }
  flox::AttrPath path = this->getAttrSetPath( ( *itr ).get<long long>( 0 ) );
  path.emplace_back( ( *itr ).get<std::string>( 1 ) );
  return path;
}


/* -------------------------------------------------------------------------- */

std::vector<row_id>
PkgDbReadOnly::getPackages( const PkgQueryArgs & params )
{
  return PkgQuery( params ).execute( this->db );
}


/* -------------------------------------------------------------------------- */

nlohmann::json
PkgDbReadOnly::getPackage( row_id row )
{
  sqlite3pp::query qry( this->db, R"SQL(
      SELECT json_object(
        'id',          Packages.id
      , 'pname',       pname
      , 'version',     version
      , 'description', Descriptions.description
      , 'license',     license
      , 'broken',      iif( ( broken IS NULL )
                          , json( 'null' )
                          , iif( broken, json( 'true' ), json( 'false' ) )
                          )
      , 'unfree',      iif( ( unfree IS NULL )
                          , json( 'null' )
                          , iif( unfree, json( 'true' ), json( 'false' ) )
                          )
      ) AS json
      FROM Packages
           LEFT JOIN Descriptions ON ( descriptionId = Descriptions.id )
           WHERE ( Packages.id = ? )
    )SQL" );
  qry.bind( 1, static_cast<long long>( row ) );

  auto rsl = nlohmann::json::parse( ( *qry.begin() ).get<std::string>( 0 ) );

  /* Add the path related field. */
  flox::AttrPath path = this->getPackagePath( row );
  rsl.emplace( "absPath", path );
  rsl.emplace( "subtree", path.at( 0 ) );
  rsl.emplace( "system", std::move( path.at( 1 ) ) );

  path.erase( path.begin(), path.begin() + 2 );
  rsl.emplace( "relPath", std::move( path ) );

  return rsl;
}


nlohmann::json
PkgDbReadOnly::getPackage( const flox::AttrPath & path )
{
  return this->getPackage( this->getPackageId( path ) );
}


/* -------------------------------------------------------------------------- */

void
DbLock::inDir( const std::filesystem::path & dir )
{
  auto lockPath
    = dir / ( this->fingerprint.to_string( nix::Base16, false ) + ".lock" );
  this->setDbLockPath( lockPath );
}


/* -------------------------------------------------------------------------- */

void
DbLock::inSameDirAs( const std::filesystem::path & file )
{
  auto lockPath = file;
  lockPath.replace_filename( this->fingerprint.to_string( nix::Base16, false )
                             + ".lock" );
  this->setDbLockPath( lockPath );
}


/* -------------------------------------------------------------------------- */

void
DbLock::release()
{
  /* Tell the heartbeat thread to stop */
  debugLog( "setting heartbeat thread shutdown flag: pid="
            + std::to_string( this->getPID() ) );
  shouldStopFlag.test_and_set( std::memory_order_seq_cst );
  /* This thread should be here if we needed to create the database. If the
   * database already existed then this will be std::nullopt. */
  if ( this->heartbeatThread.has_value() )
    {
      this->heartbeatThread->join();
      this->heartbeatThread = std::nullopt;
    }

  /* Delete the db lock */
  debugLog( "deleting the db lock: path=" + this->getDbLockPath().string() );
  try
    {
      std::filesystem::remove( this->getDbLockPath() );
    }
  catch ( const std::filesystem::filesystem_error & e )
    {
      /* Most likely it was already deleted. */
      debugLog( "failed to delete the db lock: pid="
                + std::to_string( this->getPID() ) + " msg='" + e.what()
                + "'" );
    }
}


/* -------------------------------------------------------------------------- */

DbLock::~DbLock() { this->release(); }


/* -------------------------------------------------------------------------- */

void
DbLock::spawnHeartbeatThread( std::filesystem::path db_lock,
                              DurationMillis        interval )
{
  if ( ! this->heartbeatThread.has_value() )
    {
      this->heartbeatThread
        = std::thread( periodicallyTouchDbLock, db_lock, interval );
    }
  else
    {
      // TODO: `flox` should catch this error and display a better one
      throw DbLockingException( "spawned heartbeat thread twice" );
    }
}


/* -------------------------------------------------------------------------- */

std::filesystem::path
DbLock::getDbLockPath()
{
  if ( this->dbLockPath.has_value() ) { return *( this->dbLockPath ); }
  else
    {
      this->dbLockPath = this->getDbPath();
      this->dbLockPath->replace_filename(
        this->fingerprint.to_string( nix::Base16, false ) + ".lock" );
      return *this->dbLockPath;
    }
}


/* -------------------------------------------------------------------------- */

std::filesystem::path
DbLock::getDbPath()
{
  if ( this->dbPath.has_value() ) { return *( this->dbPath ); }
  else
    {
      this->dbPath = getPkgDbCachedir().append(
        this->fingerprint.to_string( nix::Base16, false ) + ".lock" );
      return *( this->dbPath );
    }
}


/* -------------------------------------------------------------------------- */

void
DbLock::setDbLockPath( const std::filesystem::path & path )
{
  this->dbLockPath = path;
}

/* -------------------------------------------------------------------------- */

pid_t
DbLock::getPID()
{
  if ( this->pid.has_value() ) { return *( this->pid ); }
  else
    {
      auto pid  = ::getpid();
      this->pid = pid;
      return pid;
    }
}


/* -------------------------------------------------------------------------- */

std::filesystem::path
DbLock::tempDbLockPath()
{
  //
  auto path = this->getDbLockPath();
  path.replace_filename(
    "XXXXXX" );  // mkstemp replaces Xs with random characters
  auto f = ::mkstemp( (char *) path.c_str() );
  if ( f < 0 )
    {
      throw DbLockingException( "couldn't create temporary db lock",
                                std::strerror( errno ) );
    }
  ::close( f );
  /* mkstemp creates the file and we just want the path, so delete the file
   * before returning the path otherwise trying to copy to this path will fail
   * ('file exists'). */
  std::filesystem::remove( path );
  return path;
}


/* -------------------------------------------------------------------------- */

void
DbLock::writePIDsToLock( const std::vector<pid_t> & pids )
{
  /* Create a path to a non-existent temporary file, copy the contents of the
   * existing lockfile to the temp file, write to it, then mv it back. */
  auto          tmpPath = this->tempDbLockPath();
  std::ofstream tmpFile;
  tmpFile.open( tmpPath, std::ios_base::out );
  if ( ! tmpFile.good() )
    {
      throw DbLockingException( "failed to open temporary db lock copy" );
    }
  for ( auto & pid : pids ) { tmpFile << std::to_string( pid ) << std::endl; }
  std::filesystem::rename( tmpPath, this->getDbLockPath() );
  return;
}


/* -------------------------------------------------------------------------- */

std::optional<std::vector<pid_t>>
DbLock::readPIDsFromLock()
{
  std::ifstream     lock;
  std::stringstream contents;
  lock.open( this->getDbLockPath(), std::ios_base::in );
  if ( ! lock.good() ) { return std::nullopt; }
  contents << lock.rdbuf();
  lock.close();
  std::string        line;
  std::vector<pid_t> pids;
  while ( std::getline( contents, line ) )
    {
      pid_t pid = std::stoi( line );
      pids.emplace_back( pid );
      line.clear();
    }
  return pids;
}


/* -------------------------------------------------------------------------- */

void
DbLock::registerInterest()
{
  /* The write to the lock may fail due to a race condition between processes,
   * so keep trying until this PID shows up. */
  while ( true )
    {
      auto pids = this->readPIDsFromLock();
      if ( ! pids.has_value() ) { return; }
      if ( std::find( pids->begin(), pids->end(), this->getPID() )
           != pids->end() )
        {
          break;
        }
      pids->emplace_back( this->getPID() );
      this->writePIDsToLock( *pids );
    }
}


/* -------------------------------------------------------------------------- */

void
DbLock::unregisterInterest()
{
  /* The write to the lock may fail due to a race condition between processes,
   * so keep trying until this PID is gone. */
  while ( true )
    {
      auto pids = this->readPIDsFromLock();
      if ( ! pids.has_value() ) { return; }
      if ( pids->size() > 0 )
        {
          auto it = std::find( pids->begin(), pids->end(), this->getPID() );
          if ( it != pids->end() )
            {
              pids->erase( it );
              this->writePIDsToLock( *pids );
            }
          else { break; }
        }
      else { break; }
    }
}


/* -------------------------------------------------------------------------- */

DbLockActivity
DbLock::waitForLockActivity()
{
  while ( true )
    {
      /* If the lock no longer exists, return DB_LOCK_ACTIVITY_DELETED to
       * indicate that someone else created the database successfully*/
      if ( ! std::filesystem::exists( this->getDbLockPath() ) )
        {
          return DB_LOCK_ACTIVITY_DELETED;
        }
      /* The lock may get deleted between checking that it exists and checking
       * its write time.*/
      std::filesystem::file_time_type updateTime;
      try
        {
          /* If the lock still exists, check whether the last update was too
           * long ago.*/
          updateTime
            = std::filesystem::last_write_time( this->getDbLockPath() );
        }
      catch ( std::filesystem::filesystem_error & e )
        {
          return DB_LOCK_ACTIVITY_DELETED;
        }
      auto durationSinceUpdate = std::chrono::file_clock::now() - updateTime;
      if ( durationSinceUpdate > DB_LOCK_MAX_UPDATE_AGE )
        {
          return DB_LOCK_ACTIVITY_WRITER_DIED;
        }
      std::this_thread::sleep_for( DB_LOCK_TOUCH_INTERVAL );
    }
}


/* -------------------------------------------------------------------------- */

bool
DbLock::wasAbleToCreateDbLock()
{
  debugLog( "checking db lock existence: path=" + this->getDbLockPath().string()
            + " pid=" + std::to_string( this->getPID() ) );
  auto f = ::open( this->getDbLockPath().c_str(),
                   O_WRONLY | O_CREAT | O_EXCL,
                   0644 );
  if ( f < 0 )
    {
      debugLog( "couldn't create db lock: pid="
                + std::to_string( this->getPID() ) + " msg='"
                + std::strerror( errno ) + "'" );
      return false;
    }
  ::close( f );
  debugLog( "created db lock: path=" + this->getDbLockPath().string()
            + " pid=" + std::to_string( this->getPID() ) );
  return true;
}


/* -------------------------------------------------------------------------- */

DbLockState
DbLock::acquire()
{
  debugLog( "attempting to acquire db lock: pid="
            + std::to_string( this->getPID() ) );
  /* Atomically create or check that the lock exists, waiting for an in-progress
   * db operation to complete if the lock already exists. */
  if ( ! this->wasAbleToCreateDbLock() )
    {
      debugLog( "waiting for db lock to become free: pid="
                + std::to_string( this->getPID() ) );
      this->registerInterest();
      while ( true )
        {
          auto lockActivity = this->waitForLockActivity();
          /* If the lock was deleted, the database has been created and we're
           * good to read it.*/
          if ( lockActivity == DB_LOCK_ACTIVITY_DELETED )
            {
              debugLog( "detected that db lock was deleted: pid="
                        + std::to_string( this->getPID() ) );
              return DB_LOCK_FREE;
            }
          /* The lock became stale, either it's our turn to take over or we
           * should keep waiting. */
          if ( lockActivity == DB_LOCK_ACTIVITY_WRITER_DIED )
            {
              debugLog( "detected that db lock became stale: pid="
                        + std::to_string( this->getPID() ) );
              if ( this->shouldTakeOverDbCreation() )
                {
                  //
                  debugLog( "this process should take over db creation: pid="
                            + std::to_string( this->getPID() ) );
                  this->unregisterInterest();
                  return DB_LOCK_ACTION_NEEDED;
                }
            }
        }
      return DB_LOCK_FREE;
    }
  /* The lock didn't exist, which could mean the database already exists or it
   * means no one has tried to create the database yet. */
  else if ( std::filesystem::exists( this->getDbPath() ) )
    {
      debugLog( "database existed, no need to wait for lock: pid="
                + std::to_string( this->getPID() ) );
      return DB_LOCK_FREE;
    }
  /* It's up to us to create the database. */
  debugLog( "spawning heartbeat thread: pid="
            + std::to_string( this->getPID() ) );
  this->spawnHeartbeatThread( this->getDbLockPath(), DB_LOCK_TOUCH_INTERVAL );
  return DB_LOCK_FREE;
}


/* -------------------------------------------------------------------------- */

bool
DbLock::shouldTakeOverDbCreation()
{
  auto pids = this->readPIDsFromLock();
  if ( ! pids.has_value() ) { return false; }
  if ( pids->size() == 0 )
    {
      throw DbLockingException( "no PIDs found in the db lock: PID="
                                + std::to_string( this->getPID() ) );
    }
  return ( *pids )[0] == this->getPID();
}


/* -------------------------------------------------------------------------- */

void
periodicallyTouchDbLock( std::filesystem::path db_lock,
                         DurationMillis        interval )
{
  while ( ! shouldStopFlag.test( std::memory_order_seq_cst ) )
    {
      auto now = std::chrono::file_clock::now();
      try
        {
          std::filesystem::last_write_time( db_lock, now );
        }
      catch ( const std::filesystem::filesystem_error & e )
        {
          /* No one should have deleted this file. If they did, it means more
           * than one process (mistakenly) thinks it has the lock. */
          throw DbLockingException( "db lock unexpectedly missing (PID="
                                      + std::to_string( ::getpid() ) + ")",
                                    e.what() );
        }
      std::this_thread::sleep_for( interval );
    }
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
