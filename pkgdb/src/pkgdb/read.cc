/* ========================================================================== *
 *
 * @file pkgdb/read.cc
 *
 * @brief Implementations for reading a SQLite3 package set database.
 *
 *
 * -------------------------------------------------------------------------- */

#include <functional>
#include <limits>
#include <list>
#include <memory>
#include <string>
#include <unordered_set>
#include <vector>

#include "flox/core/util.hh"
#include "flox/flake-package.hh"
#include "flox/pkgdb/read.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

std::ostream &
operator<<( std::ostream & oss, const SqlVersions & versions )
{
  return oss << "tables: " << versions.tables << ", views: " << versions.views;
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
      dirname << getFloxCachedir().c_str() << "/pkgdb-v" << sqlVersions.tables;
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

void
PkgDbReadOnly::init()
{
  if ( ! std::filesystem::exists( this->dbPath ) )
    {
      throw NoSuchDatabase( *this );
    }
  this->connect();
  this->loadLockedFlake();
}


/* -------------------------------------------------------------------------- */

void
PkgDbReadOnly::connect()
{
  this->db.connect( this->dbPath.string().c_str(), SQLITE_OPEN_READONLY );
  this->db.set_busy_timeout( DB_BUSY_TIMEOUT );
}


/* -------------------------------------------------------------------------- */

void
PkgDbReadOnly::loadLockedFlake()
{
  sqlite3pp::query qry(
    this->db,
    "SELECT fingerprint, string, attrs FROM LockedFlake LIMIT 1" );
  auto rsl = qry.begin();
  if ( rsl == qry.end() )
    {
      /**
       * It is apparently possible for one process to read the scrape db
       * in just the timeframe between creation and insertion of the
       * `LockedFlake` row.
       * In that case, `qry` will return no results and we can _not_ expect to
       * be able safely dereference `rsl`.
       */
      throw PkgDbException( "No LockedFlake row." );
    }

  auto fingerprintStr = ( *rsl ).get<std::string>( 0 );

  nix::Hash fingerprint
    = nix::Hash::parseNonSRIUnprefixed( fingerprintStr, nix::htSHA256 );

  this->lockedRef.string = ( *rsl ).get<std::string>( 1 );
  this->lockedRef.attrs
    = nlohmann::json::parse( ( *rsl ).get<std::string>( 2 ) );

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

ScrapeMeta
PkgDbReadOnly::getDbScrapeMeta()
{
  sqlite3pp::query qry( this->db,
                        "SELECT value FROM DbScrapeMeta "
                        "WHERE key IN ( 'scrape_rules_hash' ) LIMIT 1" );
  auto             itr       = qry.begin();
  auto             rulesHash = ( *itr ).get<std::string>( 0 );

  return ScrapeMeta { .rulesHash = rulesHash };
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
          throw PkgDbException( nix::fmt( "No such AttrSet '%s'.",
                                          concatStringsSep( ".", path ) ) );
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
          throw PkgDbException( nix::fmt( "No such 'AttrSet.id' %llu.", row ) );
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
        nix::fmt( "No such package %s.", concatStringsSep( ".", path ) ) );
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
      throw PkgDbException( nix::fmt( "No such 'Packages.id' %llu.", row ) );
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
      , 'broken',      CASE WHEN broken IS NULL THEN json( 'null' )
                            WHEN broken         THEN json( 'true' )
                                                ELSE json( 'false' )
                       END
      , 'unfree',      CASE WHEN unfree IS NULL THEN json( 'null' )
                            WHEN unfree         THEN json( 'true' )
                                                ELSE json( 'false' )
                       END
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

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
