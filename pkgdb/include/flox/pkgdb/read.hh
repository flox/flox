/* ========================================================================== *
 *
 * @file flox/pkgdb/read.hh
 *
 * @brief Interfaces for reading a SQLite3 package set database.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <functional>
#include <queue>
#include <string>
#include <thread>
#include <vector>

#include <nix/eval-cache.hh>
#include <nix/flake/flake.hh>
#include <nlohmann/json.hpp>
#include <sqlite3pp.hh>

#include "flox/core/command.hh"
#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"
#include "flox/package.hh"


/* -------------------------------------------------------------------------- */

/* This is passed in by `make' and is set by `<pkgdb>/version' */
#ifndef FLOX_PKGDB_VERSION
#  define FLOX_PKGDB_VERSION "NO.VERSION"
#endif


/* -------------------------------------------------------------------------- */

/** @brief Interfaces for caching package metadata in SQLite3 databases. */
namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/** @brief Returns `true` if the SQLite return code indicates an error. */
inline bool
isSQLError( int rcode )
{
  switch ( rcode )
    {
      case SQLITE_OK:
      case SQLITE_ROW:
      case SQLITE_DONE: return false; break;
      default: return true; break;
    }
}

/* -------------------------------------------------------------------------- */

/** @brief Returns `true` if the SQLite database was locked during the
 * operation.*/
bool
dbIsBusy( int rcode );

/** @brief Executes the SQL command in a loop that retries when the database is
 * locked.*/
int
retryWhileBusy( sqlite3pp::command & cmd );

/** @brief Executes all SQL commands in a loop that retries when the database is
 * locked.*/
int
retryAllWhileBusy( sqlite3pp::command & cmd );

/** @brief SQLite3 schema versions. */
struct SqlVersions
{

  /**
   * The SQLite3 tables schema version for the package database.
   * Changing this value will cause the database to be recreated.
   */
  unsigned tables;

  /**
   * The SQLite3 views schema version for the package database.
   * Changing this value will cause the database's views definitions to be
   * updated, but no existing data will be invalidated.
   */
  unsigned views;

  /** @return Whether two version sets are equal. */
  constexpr bool
  operator==( const SqlVersions & other ) const
  {
    return ( this->tables == other.tables ) && ( this->views == other.views );
  }

  /** @return Whether two version sets are NOT equal. */
  constexpr bool
  operator!=( const SqlVersions & other ) const
  {
    return ! ( ( *this ) == other );
  }

  friend std::ostream &
  operator<<( std::ostream & oss, const SqlVersions & versions );

}; /* End struct `SqlVersions' */

/** @brief Emit version information to an output stream. */
std::ostream &
operator<<( std::ostream & oss, const SqlVersions & versions );


/** The current SQLite3 schema versions. */
constexpr SqlVersions sqlVersions = { .tables = 2, .views = 3 };


/* -------------------------------------------------------------------------- */

/** A unique hash associated with a locked flake. */
using Fingerprint = nix::flake::Fingerprint;
using SQLiteDb    = sqlite3pp::database; /** < SQLite3 database handle. */
using sql_rc      = int;                 /**< `SQLITE_*` result code. */


/* -------------------------------------------------------------------------- */

/**
 * @class flox::pkgdb::PkgDbException
 * @brief A generic exception thrown by `flox::pkgdb::*` classes.
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( PkgDbException, EC_PKG_DB, "error running pkgdb" )
/** @} */


/* -------------------------------------------------------------------------- */

/**
 * @brief Get the default pkgdb cache directory to save databases.
 *
 * The environment variable `PKGDB_CACHEDIR` is respected if it is set,
 * otherwise we use
 * `${XDG_CACHE_HOME:-$HOME/.cache}/flox/pkgdb-v<SCHEMA-MAJOR>`.
 */
std::filesystem::path
getPkgDbCachedir();

/** @brief Get an absolute path to the `PkgDb' for a given fingerprint hash. */
std::filesystem::path
genPkgDbName( const Fingerprint &           fingerprint,
              const std::filesystem::path & cacheDir = getPkgDbCachedir() );


/* -------------------------------------------------------------------------- */

/**
 * @brief A SQLite3 database used to cache derivation/package information about
 *        a single locked flake.
 */
class PkgDbReadOnly
{

  /* Data */

public:

  Fingerprint           fingerprint; /**< Unique hash of associated flake. */
  std::filesystem::path dbPath;      /**< Absolute path to database. */
  SQLiteDb              db;          /**< SQLite3 database handle. */

  /** @brief Locked _flake reference_ for database's flake. */
  struct LockedFlakeRef
  {
    std::string string; /**< Locked URI string.  */
    /** Exploded form of URI as an attr-set. */
    nlohmann::json attrs = nlohmann::json::object();
  };
  struct LockedFlakeRef lockedRef; /**< Locked _flake reference_. */


  /* Errors */

  // public:

  /** @brief Thrown when a database is not found. */
  struct NoSuchDatabase : PkgDbException
  {
    explicit NoSuchDatabase( const PkgDbReadOnly & pdb )
      : PkgDbException(
        std::string( "No such database '" + pdb.dbPath.string() + "'." ) )
    {}
  }; /* End struct `NoSuchDatabase' */


  /* Internal Helpers */

protected:

  /** Set @a this `PkgDb` `lockedRef` fields from database metadata. */
  void
  loadLockedFlake();


private:

  /**
   * @brief Open SQLite3 db connection at @a dbPath.
   *
   * Throw an error if no database exists.
   */
  void
  init();


  /* Constructors */

protected:

  /**
   * @brief Dummy constructor required for child classes so that they can open
   *        databases in read-only mode.
   *
   * Does NOT attempt to create a database if one does not exist.
   */
  PkgDbReadOnly() : fingerprint( nix::htSHA256 ) {}


public:

  /**
   * @brief Opens an existing database.
   *
   * Does NOT attempt to create a database if one does not exist.
   * @param dbPath Absolute path to database file.
   */
  explicit PkgDbReadOnly( std::string_view dbPath )
    : fingerprint( nix::htSHA256 ) /* Filled by `loadLockedFlake' later */
    , dbPath( dbPath )
  {
    this->init();
  }

  /**
   * @brief Opens a DB directly by its fingerprint hash.
   *
   * Does NOT attempt to create a database if one does not exist.
   * @param fingerprint Unique hash associated with locked flake.
   * @param dbPath Absolute path to database file.
   */
  PkgDbReadOnly( const Fingerprint & fingerprint, std::string_view dbPath )
    : fingerprint( fingerprint ), dbPath( dbPath )
  {
    this->init();
  }

  /**
   * @brief Opens a DB directly by its fingerprint hash.
   *
   * Does NOT attempt to create a database if one does not exist.
   * @param fingerprint Unique hash associated with locked flake.
   */
  explicit PkgDbReadOnly( const Fingerprint & fingerprint )
    : PkgDbReadOnly( fingerprint, genPkgDbName( fingerprint ).string() )
  {}


  /* Queries */

  // public:

  /** @return The Package Database schema version. */
  SqlVersions
  getDbVersion();

  /**
   * @brief Get the `AttrSet.id` for a given path.
   * @param path An attribute path prefix such as `packages.x86_64-linux` or
   *             `legacyPackages.aarch64-darwin.python3Packages`.
   * @return A unique `row_id` ( unsigned 64bit int ) associated with @a path.
   */
  row_id
  getAttrSetId( const flox::AttrPath & path );

  /**
   * @brief Check to see if database has and attribute set at @a path.
   * @param path An attribute path prefix such as `packages.x86_64-linux` or
   *             `legacyPackages.aarch64-darwin.python3Packages`.
   * @return `true` iff the database has an `AttrSet` at @a path.
   */
  bool
  hasAttrSet( const flox::AttrPath & path );

  /**
   * @brief Check to see if database has a complete list of packages under the
   *        prefix @a path.
   * @param row The `AttrSets.id` to lookup.
   * @return `true` iff the database has completely scraped the `AttrSet` at
   *          @a path.
   */
  bool
  completedAttrSet( row_id row );

  /**
   * @brief Check to see if database has a complete list of packages under the
   *        prefix @a path.
   * @param path An attribute path prefix such as `packages.x86_64-linux` or
   *             `legacyPackages.aarch64-darwin.python3Packages`.
   * @return `true` iff the database has completely scraped the `AttrSet` at
   *          @a path.
   */
  bool
  completedAttrSet( const flox::AttrPath & path );

  /**
   * @brief Get the attribute path for a given `AttrSet.id`.
   * @param row A unique `row_id` ( unsigned 64bit int ).
   * @return An attribute path prefix such as `packages.x86_64-linux` or
   *         `legacyPackages.aarch64-darwin.python3Packages`.
   */
  flox::AttrPath
  getAttrSetPath( row_id row );

  /**
   * @brief Get the `Packages.id` for a given path.
   * @param path An attribute path prefix such as
   *             `packages.x86_64-linux.hello` or
   *             `legacyPackages.aarch64-darwin.python3Packages.pip`.
   * @return A unique `row_id` ( unsigned 64bit int ) associated with @a path.
   */
  row_id
  getPackageId( const flox::AttrPath & path );

  /**
   * @brief Get the attribute path for a given `Packages.id`.
   * @param row A unique `row_id` ( unsigned 64bit int ).
   * @return An attribute path such as `packages.x86_64-linux.hello` or
   *         `legacyPackages.aarch64-darwin.python3Packages.pip`.
   */
  flox::AttrPath
  getPackagePath( row_id row );

  /**
   * @brief Check to see if database has a package at the attribute path
   *        @a path.
   * @param path An attribute path such as `packages.x86_64-linux.hello` or
   *             `legacyPackages.aarch64-darwin.python3Packages.pip`.
   * @return `true` iff the database has a rows in the `Packages`
   *         table with `path` as the _absolute path_.
   */
  bool
  hasPackage( const flox::AttrPath & path );


  /**
   * @brief Get the `Description.description` for a given `Description.id`.
   * @param descriptionId The row id to lookup.
   * @return A string describing a package.
   */
  std::string
  getDescription( row_id descriptionId );


  /**
   * @brief Return a list of `Packages.id`s for packages which satisfy a given
   *        set of requirements.
   *
   * These results may be ordered flexibly based on various query parameters.
   * TODO: document parameters effected by ordering.
   */
  std::vector<row_id>
  getPackages( const PkgQueryArgs & params );


  /**
   * @brief Get metadata about a single package.
   *
   * Returns `pname`, `version`, `description`, `broken`, `unfree`,
   * and `license` columns.
   * @param row A `Packages.id` to lookup.
   * @return A JSON object containing information about a package.
   */
  nlohmann::json
  getPackage( row_id row );


  /**
   * @brief Get metadata about a single package.
   *
   * Returns `pname`, `version`, `description`, `broken`, `unfree`,
   * and `license` columns.
   * @param row An attribute path to a package.
   * @return A JSON object containing information about a package.
   */
  nlohmann::json
  getPackage( const flox::AttrPath & path );


  nix::FlakeRef
  getLockedFlakeRef() const
  {
    return nix::FlakeRef::fromAttrs(
      nix::fetchers::jsonToAttrs( this->lockedRef.attrs ) );
  }


}; /* End class `PkgDbReadOnly' */


/* -------------------------------------------------------------------------- */

/**
 * @brief Restricts template parameters to classes that
 *        extend @a flox::pkgdb::PkgDbReadOnly.
 */
template<typename T>
concept pkgdb_typename = std::is_base_of<PkgDbReadOnly, T>::value;


/* -------------------------------------------------------------------------- */

/**
 * @brief Predicate to detect failing SQLite3 return codes.
 * @param rcode A SQLite3 _return code_.
 * @return `true` iff @a rc is a SQLite3 error.
 */
bool
isSQLError( int rcode );


/* -------------------------------------------------------------------------- */

using DurationMillis = std::chrono::duration<double, std::milli>;
const DurationMillis DB_LOCK_TOUCH_INTERVAL = DurationMillis( 100 );
/* Don't set update and check intervals to the same value, jitter in wakeup time
 * might cause flakiness
 */
const DurationMillis DB_LOCK_MAX_UPDATE_AGE = 1.5 * DB_LOCK_TOUCH_INTERVAL;

/**
 * @brief The different values that can be returned by @a DbLock::acquire.
 */
enum DbLockState {
  /* The initial state of the lock. If this is ever returned by 'acquire' that's
     a bug.*/
  DB_LOCK_INIT,
  /* You're free to do what you want with the database. */
  DB_LOCK_FREE,
  /* The database requires cleanup, but otherwise you're free to do what you
     want. */
  DB_LOCK_ACTION_NEEDED,
};

/**
 * @brief The different outcomes when monitoring the heartbeat on the db lock.
 */
enum DbLockActivity {
  /* The initial state. If this is ever returned by 'waitForLockActivity' that's
     a bug. */
  DB_LOCK_ACTIVITY_INIT,
  /* Whoever was writing the database finished writing it. */
  DB_LOCK_ACTIVITY_DELETED,
  /* The most recent lock update became stale. */
  DB_LOCK_ACTIVITY_WRITER_DIED,
};

class DbLock
{

protected:

  Fingerprint                          fingerprint;
  std::optional<std::filesystem::path> dbPath;
  std::optional<std::filesystem::path> dbLockPath;
  std::optional<pid_t>                 pid;
  std::optional<std::thread>           heartbeatThread;

  /**
   * @brief Starts a thread that touches the db lock while this lock is held.
   *
   * This will throw an exception if called more than once.
   */
  void
  spawnHeartbeatThread( std::filesystem::path db_lock,
                        DurationMillis        interval );

  /**
   * @brief Returns the PID of this process.
   */
  pid_t
  getPID();

  /**
   * @brief Atomically writes a list of PIDs to the db lock.
   *
   * Note that there may be a race condition between more than one process
   * writing their PID to the lockfile, so you need to check afterwards whether
   * the PID was actually written (e.g. the second of two atomic writes may
   * overwrite the first). We don't _really_ care which of the two processes
   * goes first, but we _do_ care that both are registered as waiting.
   */
  void
  writePIDsToLock( const std::vector<pid_t> & pids );

  /**
   * @brief Reads the PIDs in the db lock. Returns @a std::nullopt if the db
   * lock no longer exists.
   */
  std::optional<std::vector<pid_t>>
  readPIDsFromLock();

  /**
   * @brief Registers this process as waiting on the database to be created. If
   * the original writer dies the next waiter may pick up where the previous
   * writer left off.
   */
  void
  registerInterest();

  /**
   * @brief Unregister's this process as waiting on the database to be created.
   * This is mostly useful when a process is taking over database creation from
   * another process that has crashed, in which case we want the next process in
   * line to become responsible if _this_ process crashes.
   */
  void
  unregisterInterest();

  /**
   * @brief Periodically check whether the lock is still active, blocking until
   * it becomes stale or until the lock is deleted.
   *
   * Returns @a std::nullopt if the lock was deleted, indicating that the
   * database was created successfully, otherwise returns the last @a
   * std::filesystem::file_time_type at which the lock was touched.
   */
  DbLockActivity
  waitForLockActivity();

  /**
   * @brief Returns true if this process should take over creating the database.
   * This only needs to be called if @a DbLock::waitForLockActivity returned
   * something other DB_LOCK_ACTIVITY_WRITER_DIED.
   */
  bool
  shouldTakeOverDbCreation();

  /**
   * @brief Creates the database lock, returning false if it already
   * exists.
   *
   * There is a race condition between multiple processes that are launched very
   * shortly after one another. If two processes are launched at essentially the
   * same time, then they will both see that the lockfile does not exist, both
   * create the lockfile, and both spawn a heartbeat thread. Eventually one
   * process will finish and delete the lockfile. The heartbeat thread doesn't
   * expect that anyone else could delete the lockfile, so it will crash if
   * another process deletes it out from under it.
   */
  bool
  wasAbleToCreateDbLock();

public:

  DbLock( Fingerprint & fingerprint, std::filesystem::path & dbPath )
    : fingerprint( fingerprint ), dbPath( dbPath ) {};
  DbLock( Fingerprint & fingerprint ) : fingerprint( fingerprint ) {};
  DbLock( DbLock && ) = default;
  ~DbLock();
  /* TODO: make this not copyable */

  /**
   * @brief Returns the path to the db lock.
   */
  [[nodiscard]] std::filesystem::path
  getDbLockPath();

  /**
   * @brief Returns the path to the db that this lock is protecting.
   */
  [[nodiscard]] std::filesystem::path
  getDbPath();

  /**
   * @brief Set an alternative db lock path.
   *
   * Setting this means that for all lock operations the @a DbLock will look in
   * this new location for the lockfile rather than the default location, which
   * is `~/.cache/flox/pkgdb-vX/<fingerprint>.lock`.
   */
  void
  setDbLockPath( const std::filesystem::path & path );

  /**
   * @brief Use the existing fingerprint but store the lock in the provided
   * directory.
   */
  void
  inDir( const std::filesystem::path & dir );

  /**
   * @brief Use the existing fingerprint but store the lock in the same parent
   * directory as the provided file.
   */
  void
  inSameDirAs( const std::filesystem::path & file );

  /** @brief Blocks until the lock can be acquired.
   *
   * The return value is a @a flox::pkgdb::DbLockState. This function should
   * only ever return @a DB_LOCK_FREE or @a DB_LOCK_ACTION_NEEDED. The
   * `DB_LOCK_FREE` value indicates that the database was already created and
   * you don't need to recreate it. The `DB_LOCK_ACTION_NEEDED` value indicates
   * that the original writer crashed while creating the database and it's now
   * your responsibility to create it.
   */
  DbLockState
  acquire();

  /**
   * @brief Releases the lock by terminating the heartbeat thread and deleting
   * the db lock.
   */
  void
  release();
};

/**
 * @brief Periodically touches the db lock.
 *
 * Meant to be called from a separate thread as it will never return.
 */
void
periodicallyTouchDbLock( const std::filesystem::path db_lock,
                         const DurationMillis        interval );

/**
 * Process A checks for the existence of <fingerprint>.lock
 * Process A sees that it doesn't exist
 * Process A checks for existence of the <fingerprint>.sqlite
 * Process A sees that it doesn't exist
 * Process A creates <fingerprint>.lock
 * Process A creates <fingerprint>.sqlite
 * Process A starts a thread that periodically touches <fingerprint>.lock
 * Process A begins writing to the database
 * Process A deletes <fingerprint.lock> when it's done constructing the database
 *
 * Process B is launched after Process A
 * Process B checks for existence of <fingerprint>.lock
 * Process B sees that it exists
 * Process B appends its PID to the lockfile
 * In a loop:
 *  Process B tries to read the mtime of <fingerprint>.lock
 *  If the file doesn't exist, it proceeds to read the database
 *  If the file does exist, it checks whether the mtime was within some interval
 * from the past If the mtime was within this interval, the original writer must
 * still be alive sleep for some period If the mtime wasn't within the interval,
 * the original writer died Process B reads the first line of the file If it
 * matches its own PID then it gets to take control
 */


/**
 * @class flox::DbLockingException
 * @brief An exception thrown when locking a package database.
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( DbLockingException,
                       EC_DB_LOCKING,
                       "error locking package database" )
/** @} */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
