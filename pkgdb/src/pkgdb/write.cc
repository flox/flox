/* ========================================================================== *
 *
 * @file pkgdb/write.cc
 *
 * @brief Implementations for writing to a SQLite3 package set database.
 *
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <limits>
#include <memory>
#include <optional>
#include <string>

#include <nlohmann/json.hpp>

#include "flox/core/util.hh"
#include "flox/flake-package.hh"
#include "flox/pkgdb/write.hh"

#include "./schemas.hh"

/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/** @brief Create views in database if they do not exist. */
static void
initViews( SQLiteDb & pdb )
{
  sqlite3pp::command cmd( pdb, sql_views );
  if ( sql_rc rcode = cmd.execute_all(); isSQLError( rcode ) )
    {
      throw PkgDbException( nix::fmt( "failed to initialize views:(%d) %s",
                                      rcode,
                                      pdb.error_msg() ) );
    }
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Update the database's `VIEW`s schemas.
 *
 * This deletes any existing `VIEW`s and recreates them, and updates the
 * `DbVersions` row for `pkgdb_views_schema`.
 */
static void
updateViews( SQLiteDb & pdb )
{
  /* Drop all existing views. */
  {
    sqlite3pp::query qry( pdb,
                          "SELECT name FROM sqlite_master WHERE"
                          " ( type = 'view' )" );
    for ( auto row : qry )
      {
        auto               name = row.get<std::string>( 0 );
        std::string        cmd  = "DROP VIEW IF EXISTS '" + name + '\'';
        sqlite3pp::command dropView( pdb, cmd.c_str() );
        if ( sql_rc rcode = dropView.execute(); isSQLError( rcode ) )
          {
            throw PkgDbException( nix::fmt( "failed to drop view '%s':(%d) %s",
                                            name,
                                            rcode,
                                            pdb.error_msg() ) );
          }
      }
  }

  /* Update the `pkgdb_views_schema' version. */
  sqlite3pp::command updateVersion(
    pdb,
    "UPDATE DbVersions SET version = ? WHERE name = 'pkgdb_views_schema'" );
  updateVersion.bind( 1, static_cast<int>( sqlVersions.views ) );

  if ( sql_rc rcode = updateVersion.execute_all(); isSQLError( rcode ) )
    {
      throw PkgDbException( nix::fmt( "failed to update PkgDb Views:(%d) %s",
                                      rcode,
                                      pdb.error_msg() ) );
    }

  /* Redefine the `VIEW's */
  initViews( pdb );
}


/* -------------------------------------------------------------------------- */

/** @brief Create tables in database if they do not exist. */
static void
initTables( SQLiteDb & pdb )
{
  sqlite3pp::command cmdVersions( pdb, sql_versions );
  if ( sql_rc rcode = cmdVersions.execute(); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to initialize DbVersions table:(%d) %s",
                  rcode,
                  pdb.error_msg() ) );
    }

  sqlite3pp::command cmdInput( pdb, sql_input );
  if ( sql_rc rcode = cmdInput.execute_all(); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to initialize LockedFlake table:(%d) %s",
                  rcode,
                  pdb.error_msg() ) );
    }

  sqlite3pp::command cmdAttrSets( pdb, sql_attrSets );
  if ( sql_rc rcode = cmdAttrSets.execute_all(); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to initialize AttrSets table:(%d) %s",
                  rcode,
                  pdb.error_msg() ) );
    }

  sqlite3pp::command cmdPackages( pdb, sql_packages );
  if ( sql_rc rcode = cmdPackages.execute_all(); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to initialize Packages table:(%d) %s",
                  rcode,
                  pdb.error_msg() ) );
    }
}


/* -------------------------------------------------------------------------- */

/** @brief Create `DbVersions` rows if they do not exist. */
static void
initVersions( SQLiteDb & pdb )
{
  sqlite3pp::command defineVersions(
    pdb,
    "INSERT OR IGNORE INTO DbVersions ( name, version ) VALUES"
    "  ( 'pkgdb',        '" FLOX_PKGDB_VERSION "' )"
    ", ( 'pkgdb_tables_schema', ? )"
    ", ( 'pkgdb_views_schema', ? )" );
  defineVersions.bind( 1, static_cast<int>( sqlVersions.tables ) );
  defineVersions.bind( 2, static_cast<int>( sqlVersions.views ) );
  if ( sql_rc rcode = defineVersions.execute(); isSQLError( rcode ) )
    {
      throw PkgDbException( "failed to write DbVersions info",
                            pdb.error_msg() );
    }
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Update the database's `VIEW`s schemas.
 *
 * This deletes any existing `VIEW`s and recreates them, and updates the
 * `DbVersions` row for `pkgdb_views_schema`.
 */
static void
updateViews( PkgDb & pdb )
{
  initTables( this->db );
  initVersions( this->db );

  /* If the views version is outdated, update them. */
  if ( this->getDbVersion().views < sqlVersions.views )
    {
      updateViews( this->db );
    }
  else { initViews( this->db ); }
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Write @a this `PkgDb` `lockedRef` and `fingerprint` fields to
 *        database metadata.
 */
static void
writeInput( PkgDb & pdb )
{
  sqlite3pp::command cmd(
    pdb.db,
    "INSERT OR IGNORE INTO LockedFlake ( fingerprint, string, attrs ) VALUES"
    "  ( ?, ?, ? )" );
  std::string fpStr = pdb.fingerprint.to_string( nix::Base16, false );
  cmd.bind( 1, fpStr, sqlite3pp::copy );
  cmd.bind( 2, pdb.lockedRef.string, sqlite3pp::nocopy );
  cmd.bind( 3, pdb.lockedRef.attrs.dump(), sqlite3pp::copy );
  if ( sql_rc rcode = cmd.execute(); isSQLError( rcode ) )
    {
      throw PkgDbException( "failed to write LockedFlaked info",
                            pdb.db.error_msg() );
    }
}


/* -------------------------------------------------------------------------- */

PkgDb::PkgDb( const nix::flake::LockedFlake & flake, std::string_view dbPath )
{
  this->dbPath      = dbPath;
  this->fingerprint = flake.getFingerprint();
  this->connect();
  this->init();
  this->lockedRef
    = { flake.flake.lockedRef.to_string(),
        nix::fetchers::attrsToJSON( flake.flake.lockedRef.toAttrs() ) };
  writeInput( *this );
}


/* -------------------------------------------------------------------------- */

void
PkgDb::connect()
{
  /* The `locking_mode` pragma acquires an exclusive write lock the first time
   * that the database is written to and only releases the lock once the
   * database connection is closed. We make a write as soon as possible after
   * opening the connection to establish the write lock. After the
   * `RETRY_WHILE_BUSY` call returns we should be the only process able to write
   * to the database, though other processes mays still read from the database
   * (this is why we must use `EXCLUSIVE` transactions, which prevent reads as
   * well).
   *
   * It could be the case that this database hasn't been initialized yet, so we
   * can't write to an existing table. Instead we just write to a dummy table.
   * It's unclear whether setting a pragma value like `appliation_id` counts as
   * a write, so we create a table instead.*/
  static const char * acquire_lock = R"SQL(
  BEGIN EXCLUSIVE TRANSACTION;
  CREATE TABLE IF NOT EXISTS _lock (foo int);
  COMMIT TRANSACTION
  )SQL";
  this->db.connect( this->dbPath.string().c_str(),
                    SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE );
  sqlite3pp::command cmd( this->db, acquire_lock );
  RETRY_WHILE_BUSY( cmd.execute_all() );
}


/* -------------------------------------------------------------------------- */

/** @brief Write a rules hash to a database. */
static void
writeScrapeRulesHash( SQLiteDb & database, const RulesTreeNode & rules )
{
  sqlite3pp::command cmd(
    database,
    "INSERT OR IGNORE INTO ScrapeRules ( hash ) VALUES ( ? )" );
  cmd.bind( 1, rules.getHash(), sqlite3pp::copy );
  if ( sql_rc rcode = cmd.execute(); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to write ScrapeRules hash:(%d) %s",
                  rcode,
                  database.error_msg() ) );
    }
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Clear all rows from tables effected by rules changes.
 *
 * This includes `ScrapeRules`, `AttrSets`, `Descriptions`, and `Packages`.
 */
static sql_rc
clearDbTables( PkgDb & pdb )
{
  return pdb.execute_all( R"SQL(
    DELETE FROM TABLE ScrapeRules;
    DELETE FROM TABLE Descriptions;
    DELETE FROM TABLE Packages;
    DELETE FROM TABLE AttrSets
  )SQL" );
}


/* -------------------------------------------------------------------------- */

void
PkgDb::init()
{
  initTables( *this );

  // TODO: Rules should /really/ be associated with inputs.
  //       This initialization belongs closer to `writeInput()'.
  //       The reason this is okay today is because we only allow `nixpkgs'.
  /* Detect mismatched on uninitialized `ScrapeRules.hash'. */
  try
    {
      (void) this->getRules();
    }
  catch ( const RulesHashMismatch & )
    {
      debugLog( "clearing database `" + this->dbPath.string()
                + "' with stale rules." );
      clearDbTables( *this );
    }
  catch ( const RulesHashMissing & )
    {
      traceLog( "writing rules hash to `" + this->dbPath.string() + "'" );
      /* Indicates uninitialized. */
      writeScrapeRulesHash( this->db, this->rules.value() );
    }

  initVersions( *this );

  /* If the views version is outdated, update them. */
  if ( this->getDbVersion().views < sqlVersions.views )
    {
      updateViews( *this );
    }
  else { initViews( *this ); }
}


/* -------------------------------------------------------------------------- */

row_id
PkgDb::addOrGetAttrSetId( const std::string & attrName, row_id parent )
{
  sqlite3pp::command cmd(
    this->db,
    "INSERT INTO AttrSets ( attrName, parent ) VALUES ( ?, ? )" );
  cmd.bind( 1, attrName, sqlite3pp::copy );
  cmd.bind( 2, static_cast<long long>( parent ) );
  if ( sql_rc rcode = cmd.execute(); isSQLError( rcode ) )
    {
      sqlite3pp::query qryId(
        this->db,
        "SELECT id FROM AttrSets WHERE ( attrName = ? ) AND ( parent = ? )" );
      qryId.bind( 1, attrName, sqlite3pp::copy );
      qryId.bind( 2, static_cast<long long>( parent ) );
      auto row = qryId.begin();
      if ( row == qryId.end() )
        {
          throw PkgDbException(
            nix::fmt( "failed to add AttrSet.id `AttrSets[%ull].%s':(%d) %s",
                      parent,
                      attrName,
                      rcode,
                      this->db.error_msg() ) );
        }
      return ( *row ).get<long long>( 0 );
    }
  return this->db.last_insert_rowid();
}


/* -------------------------------------------------------------------------- */

row_id
PkgDb::addOrGetAttrSetId( const flox::AttrPath & path )
{
  row_id row = 0;
  for ( const auto & attr : path ) { row = addOrGetAttrSetId( attr, row ); }
  return row;
}


/* -------------------------------------------------------------------------- */

row_id
PkgDb::addOrGetDescriptionId( const std::string & description )
{
  sqlite3pp::query qry(
    this->db,
    "SELECT id FROM Descriptions WHERE description = ? LIMIT 1" );
  qry.bind( 1, description, sqlite3pp::copy );
  auto rows = qry.begin();
  if ( rows != qry.end() )
    {
      nix::Activity act(
        *nix::logger,
        nix::lvlDebug,
        nix::actUnknown,
        nix::fmt( "Found existing description in database: %s.",
                  description ) );
      return ( *rows ).get<long long>( 0 );
    }

  sqlite3pp::command cmd(
    this->db,
    "INSERT INTO Descriptions ( description ) VALUES ( ? )" );
  cmd.bind( 1, description, sqlite3pp::copy );
  nix::Activity act(
    *nix::logger,
    nix::lvlDebug,
    nix::actUnknown,
    nix::fmt( "Adding new description to database: %s.", description ) );
  if ( sql_rc rcode = cmd.execute(); isSQLError( rcode ) )
    {
      throw PkgDbException( nix::fmt( "failed to add Description '%s':(%d) %s",
                                      description,
                                      rcode,
                                      this->db.error_msg() ) );
    }
  return this->db.last_insert_rowid();
}


/* -------------------------------------------------------------------------- */

row_id
PkgDb::addPackage( row_id               parentId,
                   std::string_view     attrName,
                   const flox::Cursor & cursor,
                   bool                 replace,
                   bool                 checkDrv )
{
#define ADD_PKG_BODY                                                   \
  " INTO Packages ("                                                   \
  "  parentId, attrName, name, pname, version, semver, license"        \
  ", outputs, outputsToInstall, broken, unfree, descriptionId"         \
  ") VALUES ("                                                         \
  "  :parentId, :attrName, :name, :pname, :version, :semver, :license" \
  ", :outputs, :outputsToInstall, :broken, :unfree, :descriptionId"    \
  ")"
  static const char * qryIgnore  = "INSERT OR IGNORE" ADD_PKG_BODY;
  static const char * qryReplace = "INSERT OR REPLACE" ADD_PKG_BODY;

  sqlite3pp::command cmd( this->db, replace ? qryReplace : qryIgnore );

  /* We don't need to reference any `attrPath' related info here, so
   * we can avoid looking up the parent path by passing a phony one to the
   * `FlakePackage' constructor here. */
  FlakePackage pkg( cursor, { "packages", "x86_64-linux", "phony" }, checkDrv );
  std::string  attrNameS( attrName );

  cmd.bind( ":parentId", static_cast<long long>( parentId ) );
  cmd.bind( ":attrName", attrNameS, sqlite3pp::copy );
  cmd.bind( ":name", pkg._fullName, sqlite3pp::nocopy );
  cmd.bind( ":pname", pkg._pname, sqlite3pp::nocopy );

  if ( pkg._version.empty() ) { cmd.bind( ":version" ); /* bind NULL */ }
  else { cmd.bind( ":version", pkg._version, sqlite3pp::nocopy ); }

  if ( pkg._semver.has_value() )
    {
      cmd.bind( ":semver", *pkg._semver, sqlite3pp::nocopy );
    }
  else { cmd.bind( ":semver" ); /* binds NULL */ }

  {
    nlohmann::json jOutputs = pkg.getOutputs();
    cmd.bind( ":outputs", jOutputs.dump(), sqlite3pp::copy );
  }
  {
    nlohmann::json jOutsInstall = pkg.getOutputsToInstall();
    cmd.bind( ":outputsToInstall", jOutsInstall.dump(), sqlite3pp::copy );
  }


  if ( pkg._hasMetaAttr )
    {
      if ( auto maybe = pkg.getLicense(); maybe.has_value() )
        {
          cmd.bind( ":license", *maybe, sqlite3pp::copy );
        }
      else { cmd.bind( ":license" ); }

      if ( auto maybe = pkg.isBroken(); maybe.has_value() )
        {
          cmd.bind( ":broken", static_cast<int>( *maybe ) );
        }
      else { cmd.bind( ":broken" ); }

      if ( auto maybe = pkg.isUnfree(); maybe.has_value() )
        {
          cmd.bind( ":unfree", static_cast<int>( *maybe ) );
        }
      else /* TODO: Derive value from `license'? */ { cmd.bind( ":unfree" ); }

      if ( auto maybe = pkg.getDescription(); maybe.has_value() )
        {
          row_id descriptionId = this->addOrGetDescriptionId( *maybe );
          cmd.bind( ":descriptionId", static_cast<long long>( descriptionId ) );
        }
      else { cmd.bind( ":descriptionId" ); }
    }
  else
    {
      /* binds NULL */
      cmd.bind( ":license" );
      cmd.bind( ":broken" );
      cmd.bind( ":unfree" );
      cmd.bind( ":descriptionId" );
    }
  if ( sql_rc rcode = cmd.execute(); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to write Package '%s'", pkg._fullName ),
        this->db.error_msg() );
    }
  return this->db.last_insert_rowid();
}


/* -------------------------------------------------------------------------- */

void
PkgDb::setPrefixDone( row_id prefixId, bool done )
{
  sqlite3pp::command cmd( this->db, R"SQL(
    UPDATE AttrSets SET done = ? WHERE id in (
      WITH RECURSIVE Tree AS (
        SELECT id, parent, 0 as depth FROM AttrSets
        WHERE ( id = ? )
        UNION ALL SELECT O.id, O.parent, ( Parent.depth + 1 ) AS depth
        FROM AttrSets O
        JOIN Tree AS Parent ON ( Parent.id = O.parent )
      ) SELECT C.id FROM Tree AS C
      JOIN AttrSets AS Parent ON ( C.parent = Parent.id )
    )
  )SQL" );
  cmd.bind( 1, static_cast<int>( done ) );
  cmd.bind( 2, static_cast<long long>( prefixId ) );
  if ( sql_rc rcode = cmd.execute(); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to set AttrSets.done for subtree '%s':(%d) %s",
                  concatStringsSep( ".", this->getAttrSetPath( prefixId ) ),
                  rcode,
                  this->db.error_msg() ) );
    }
}

void
PkgDb::setPrefixDone( const flox::AttrPath & prefix, bool done )
{
  this->setPrefixDone( this->addOrGetAttrSetId( prefix ), done );
}


/* -------------------------------------------------------------------------- */

/* NOTE:
 * Benchmarks on large catalogs have indicated that using a _todo_ queue instead
 * of recursion is faster and consumes less memory.
 * Repeated runs against `nixpkgs-flox` come in at ~2m03s using recursion and
 * ~1m40s using a queue. */
void
PkgDb::scrape( nix::SymbolTable & syms, const Target & target, Todos & todo )
{
  const auto & [prefix, cursor, parentId] = target;

  /* If it has previously been scraped then bail out. */
  if ( this->completedAttrSet( parentId ) ) { return; }

  debugLog( nix::fmt( "evaluating package set '%s'",
                      concatStringsSep( ".", prefix ) ) );

  /* Scrape loop over attrs */
  for ( nix::Symbol & aname : cursor->getAttrs() )
    {
      /* Used for logging, but can skip it at low verbosity levels. */
      const std::string pathS
        = ( nix::lvlTalkative <= nix::verbosity )
            ? concatStringsSep( ".", prefix ) + "." + syms[aname]
            : "";

      /* We know this isn't a package or an attrset, so skip immediately. */
      if ( syms[aname] == "recurseForDerivations" )
        {
          traceLog( "skipping keyword attribute: " + pathS );
          continue;
        }

      /* Skip anything with a "__" prefix. */
      if ( hasPrefix( "__", static_cast<std::string_view>( syms[aname] ) ) )
        {
          traceLog( "skipping attribute with \"__\" prefix: " + pathS );
          continue;
        }

      traceLog( "\tevaluating attribute '" + pathS + "'" );

      AttrPath path( prefix );
      path.emplace_back( syms[aname] );

      std::optional<bool> rulesAllowed = this->getRules().applyRules( path );

      // FIXME: This breaks allows under recursiveDisallows!
      /* If the package or prefix is disallowed, bail. */
      if ( rulesAllowed.has_value() && ( ! ( *rulesAllowed ) ) )
        {
          traceLog( "skipping disallowed attribute: " + pathS );
          continue;
        }
      try
        {
          flox::Cursor child = cursor->getAttr( aname );
          if ( child->isDerivation() )
            {
              traceLog( "adding derivation to DB: " + pathS );
              this->addPackage( parentId, syms[aname], child );
              continue;
            }

          /* Ensure that it's an attribute set AND that it is not a functor. */
          try
            {
              std::vector<nix::Symbol> maybeAttrs = child->getAttrs();
              if ( maybeAttrs.empty() )
                {
                  traceLog( "skipping empty attribute set: " + pathS );
                  continue;
                }
              for ( const auto & symbol : maybeAttrs )
                {
                  if ( ( syms[symbol] == "__functor" )
                       || ( syms[symbol] == "__functionArgs" ) )
                    {
                      traceLog( "skipping functor: " + pathS );
                      throw nix::EvalError( "attribute set is a functor" );
                    }
                }
            }
          catch ( const nix::EvalError & err )
            {
              /* It wasn't... */
              traceLog( "skipping attribute set with eval error: " + pathS );
              continue;
            }

          auto maybe           = child->maybeGetAttr( "recurseForDerivations" );
          bool markedRecursive = ( maybe != nullptr ) && maybe->getBool();

          if ( rulesAllowed.value_or( markedRecursive ) )
            {
              printLog( nix::lvlTalkative,
                        "\tpushing target '" + pathS + '\'' );
              row_id childId = this->addOrGetAttrSetId( syms[aname], parentId );
              todo.emplace( std::make_tuple( std::move( path ),
                                             std::move( child ),
                                             childId ) );
            }
        }
      catch ( const nix::EvalError & err )
        {
          /* Only treat errors as fatal in the `packages' sub-tree. */
          if ( prefix.front() != "packages" )
            {
              traceLog( "skipping attribute set with eval error: " + pathS );
              /* Only print eval errors in "debug" mode. */
              nix::ignoreException( nix::lvlDebug );
            }
          else { throw; }
        }
    }
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
