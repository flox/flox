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
#include "flox/pkgdb/pkg-query.hh"
#include <nix/fetchers.hh>
#include <nix/url.hh>


/* -------------------------------------------------------------------------- */

/* This is passed in by `make' and is set by `<pkgdb>/version' */
#ifndef FLOX_PKGDB_VERSION
#  define FLOX_PKGDB_VERSION "NO.VERSION"
#endif


/* -------------------------------------------------------------------------- */

/** @brief Interfaces for caching package metadata in SQLite3 databases. */
namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/* We may need to wait for the database to be constructed, and that could take
 * some time. We set a reasonably small retry period to preserve responsiveness,
 * but set a large number of retries so that a slow database operation isn't
 * terminated too early. */
const DurationMillis DB_RETRY_PERIOD = DurationMillis( 100 );
const int            DB_MAX_RETRIES  = 2500;

#define RETRY_WHILE_BUSY( op )                                    \
  int _retry_while_busy_rcode   = op;                             \
  int _retry_while_busy_retries = 0;                              \
  while ( _retry_while_busy_rcode == SQLITE_BUSY )                \
    {                                                             \
      if ( ++_retry_while_busy_retries > DB_MAX_RETRIES )         \
        {                                                         \
          throw PkgDbException( "database operation timed out" ); \
        }                                                         \
      std::this_thread::sleep_for( DB_RETRY_PERIOD );             \
      _retry_while_busy_rcode = op;                               \
    }


/* -------------------------------------------------------------------------- */

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

  /* Connecting and locking */

  /**
   * @brief Tries to connect to the database.
   *
   * The database may be locked by another process that is currently scraping
   * it. This function will block until that lock is released. Will not acquire
   * an exclusive lock on the database so that other process can concurrently
   * read the database.
   */
  void
  connect();

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


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
