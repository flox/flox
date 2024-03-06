/* ========================================================================== *
 *
 * @file pkgdb.cc
 *
 * @brief Tests for `flox::pkgdb::PkgDb` interfaces.
 *
 * NOTE: These tests may be order dependant simply because each test case shares
 *       a single database.
 *       Having said that we make a concerted effort to avoid dependence on past
 *       test state by doing things like clearing tables in test cases where
 *       it may be relevant to an action we're about to test.
 *
 * In general tests should clear the database's tables at the top of
 * their function.
 * This allows `throw` and early terminations to exit at arbitrary points
 * without polluting later test cases.
 *
 *
 * -------------------------------------------------------------------------- */

#include <assert.h>
#include <cstdlib>
#include <iostream>
#include <limits>
#include <list>
#include <queue>

#include <nix/eval-cache.hh>
#include <nix/eval.hh>
#include <nix/flake/flake.hh>
#include <nix/shared.hh>
#include <nix/store-api.hh>
#include <sqlite3pp.hh>

#include "flox/core/nix-state.hh"
#include "flox/core/types.hh"
#include "flox/flox-flake.hh"
#include "flox/pkgdb/db-package.hh"
#include "flox/pkgdb/input.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/pkgdb/scrape-rules.hh"
#include "flox/pkgdb/write.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

using flox::pkgdb::row_id;

/* -------------------------------------------------------------------------- */

static const nlohmann::json pkgDescriptorBaseRaw = R"( {
  "name": "name",
  "pname": "pname",
  "version": "version",
  "semver": "semver"
} )"_json;

/* -------------------------------------------------------------------------- */

static const std::string_view rulesJSON = R"( {
  "allowRecursive": [
    ["legacyPackages", null, "darwin"],
    ["legacyPackages", null, "swiftPackages", "darwin"]
  ],
  "disallowRecursive": [
    ["legacyPackages", null, "emacsPackages"],
    ["legacyPackages", null, "python310Packages"]
  ],
 "allowPackage": [
   ["legacyPackages", null, "python310Packages", "pip"]
 ],
 "disallowPackage": [
   ["legacyPackages", null, "gcc"]
 ]
} )";

/* -------------------------------------------------------------------------- */

static row_id
getRowCount( flox::pkgdb::PkgDb & db, const std::string table )
{
  std::string qryS = "SELECT COUNT( * ) FROM ";
  qryS += table;
  sqlite3pp::query qry( db.db, qryS.c_str() );
  return ( *qry.begin() ).get<long long int>( 0 );
}


/* -------------------------------------------------------------------------- */

static inline void
clearTables( flox::pkgdb::PkgDb & db )
{
  /* Clear DB */
  db.execute_all(
    "DELETE FROM Packages; DELETE FROM AttrSets; DELETE FROM Descriptions" );
}

/* -------------------------------------------------------------------------- */

/**
 * Test ability to add `AttrSet` rows.
 * This test should run before all others since it essentially expects
 * `AttrSets` to be empty.
 */
bool
test_addOrGetAttrSetId0( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make sure `AttrSets` is empty. */
  row_id startId = getRowCount( db, "AttrSets" );
  EXPECT_EQ( startId, static_cast<row_id>( 0 ) );

  /* Add two `AttrSets` */
  row_id id = db.addOrGetAttrSetId( "legacyPackages" );
  EXPECT_EQ( startId + 1, id );

  id = db.addOrGetAttrSetId( "x86_64-linux", id );
  EXPECT_EQ( startId + 2, id );

  return true;
}


/* -------------------------------------------------------------------------- */

/** Ensure we throw an error for undefined `AttrSet.id' parents. */
bool
test_addOrGetAttrSetId1( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  try
    {
      /* Ensure we throw an error for undefined `AttrSet.id' parents. */
      db.addOrGetAttrSetId( "phony", 1 );
      return false;
    }
  catch ( const flox::pkgdb::PkgDbException & e )
    { /* Expected */
    }
  catch ( const std::exception & e )
    {
      std::cerr << e.what() << std::endl;
      return false;
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/** Ensure database version matches our header's version */
bool
test_getDbVersion0( flox::pkgdb::PkgDb & db )
{
  EXPECT_EQ( db.getDbVersion(), flox::pkgdb::sqlVersions );
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * Ensure `PkgDb::hasAttrSet` works regardless of whether `Packages` exist in
 * an `AttrSet`.
 */
bool
test_hasAttrSet0( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make sure the attr-set exists, and clear it. */
  row_id             id = db.addOrGetAttrSetId( "x86_64-linux",
                                    db.addOrGetAttrSetId( "legacyPackages" ) );
  sqlite3pp::command cmd( db.db,
                          "DELETE FROM Packages WHERE ( parentId = :id )" );
  cmd.bind( ":id", static_cast<long long int>( id ) );
  cmd.execute();

  EXPECT( db.hasAttrSet(
    std::vector<std::string> { "legacyPackages", "x86_64-linux" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * Ensure `PkgDb::hasAttrSet` works when `Packages` exist in an `AttrSet`
 * such that attribute sets with packages are identified as "Package Sets".
 */
bool
test_hasAttrSet1( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make sure the attr-set exists. */
  row_id id = db.addOrGetAttrSetId( "x86_64-linux",
                                    db.addOrGetAttrSetId( "legacyPackages" ) );
  /* Add a minimal package with this `id` as its parent. */
  sqlite3pp::command cmd( db.db, R"SQL(
      INSERT OR IGNORE INTO Packages ( parentId, attrName, name, outputs )
      VALUES ( :id, 'phony', 'phony', '["out"]' )
    )SQL" );
  cmd.bind( ":id", static_cast<long long>( id ) );
  cmd.execute();

  EXPECT( db.hasAttrSet(
    std::vector<std::string> { "legacyPackages", "x86_64-linux" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * Ensure the `row_id` returned when adding an `AttrSet` matches the one
 * returned by @a flox::pkgdb::PkgDb::getAttrSetId.
 */
bool
test_getAttrSetId0( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make sure the attr-set exists. */
  row_id id = db.addOrGetAttrSetId( "x86_64-linux",
                                    db.addOrGetAttrSetId( "legacyPackages" ) );
  EXPECT_EQ( id,
             db.getAttrSetId( std::vector<std::string> { "legacyPackages",
                                                         "x86_64-linux" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * Ensure we properly reconstruct an attribute path from the `AttrSets` table.
 */
bool
test_getAttrSetPath0( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make sure the attr-set exists. */
  row_id                   id = db.addOrGetAttrSetId( "x86_64-linux",
                                    db.addOrGetAttrSetId( "legacyPackages" ) );
  std::vector<std::string> path { "legacyPackages", "x86_64-linux" };
  EXPECT( path == db.getAttrSetPath( id ) );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_hasPackage0( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make sure the attr-set exists. */
  row_id id = db.addOrGetAttrSetId( "x86_64-linux",
                                    db.addOrGetAttrSetId( "legacyPackages" ) );
  /* Add a minimal package with this `id` as its parent. */
  sqlite3pp::command cmd( db.db, R"SQL(
      INSERT OR IGNORE INTO Packages ( parentId, attrName, name, outputs )
      VALUES ( :id, 'phony', 'phony', '["out"]' )
    )SQL" );
  cmd.bind( ":id", static_cast<long long>( id ) );
  cmd.execute();

  EXPECT( db.hasPackage(
    flox::AttrPath { "legacyPackages", "x86_64-linux", "phony" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * Tests `addOrGetDesciptionId` and `getDescription`.
 */
bool
test_descriptions0( flox::pkgdb::PkgDb & db )
{
  row_id id = db.addOrGetDescriptionId( "Hello, World!" );
  /* Ensure we get the same `id`. */
  EXPECT_EQ( id, db.addOrGetDescriptionId( "Hello, World!" ) );
  /* Ensure we get back our original string. */
  EXPECT_EQ( "Hello, World!", db.getDescription( id ) );
  return true;
}

/* -------------------------------------------------------------------------- */

/* Tests `systems', `name', `pname', `version', and `subtree' filtering. */
bool
test_PkgQuery0( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make a package */
  row_id linux = db.addOrGetAttrSetId(
    flox::AttrPath { "legacyPackages", "x86_64-linux" } );
  row_id desc
    = db.addOrGetDescriptionId( "A program with a friendly greeting" );
  sqlite3pp::command cmd( db.db, R"SQL(
    INSERT INTO Packages (
      parentId, attrName, name, pname, version, semver, outputs, descriptionId
    ) VALUES ( :parentId, 'hello', 'hello-2.12.1', 'hello', '2.12.1', '2.12.1'
             , '["out"]', :descriptionId
             )
  )SQL" );
  cmd.bind( ":parentId", static_cast<long long>( linux ) );
  cmd.bind( ":descriptionId", static_cast<long long>( desc ) );
  if ( flox::pkgdb::sql_rc rc = cmd.execute(); flox::isSQLError( rc ) )
    {
      throw flox::pkgdb::PkgDbException(
        nix::fmt( "Failed to write Package 'hello':(%d) %s",
                  rc,
                  db.db.error_msg() ) );
    }
  flox::pkgdb::PkgQueryArgs qargs;
  qargs.systems = std::vector<std::string> { "x86_64-linux" };

  /* Run empty query */
  {
    flox::pkgdb::PkgQuery            query( qargs );
    std::vector<flox::pkgdb::row_id> rsl = query.execute( db.db );
    EXPECT( ( rsl.size() == 1 ) && ( 0 < rsl.at( 0 ) ) );
  }

  /* Run `pname' query */
  {
    qargs.pname = "hello";
    flox::pkgdb::PkgQuery query( qargs );
    qargs.pname                          = std::nullopt;
    std::vector<flox::pkgdb::row_id> rsl = query.execute( db.db );
    EXPECT( ( rsl.size() == 1 ) && ( 0 < rsl.at( 0 ) ) );
  }

  /* Run `version' query */
  {
    qargs.version = "2.12.1";
    flox::pkgdb::PkgQuery query( qargs );
    qargs.version                        = std::nullopt;
    std::vector<flox::pkgdb::row_id> rsl = query.execute( db.db );
    EXPECT( ( rsl.size() == 1 ) && ( 0 < rsl.at( 0 ) ) );
  }

  /* Run `name' query */
  {
    qargs.name = "hello-2.12.1";
    flox::pkgdb::PkgQuery query( qargs );
    qargs.name                           = std::nullopt;
    std::vector<flox::pkgdb::row_id> rsl = query.execute( db.db );
    EXPECT( ( rsl.size() == 1 ) && ( 0 < rsl.at( 0 ) ) );
  }

  /* Run `subtrees' query */
  {
    qargs.subtrees = std::vector<flox::Subtree> { flox::ST_LEGACY };
    flox::pkgdb::PkgQuery query( qargs );
    qargs.subtrees                       = std::nullopt;
    std::vector<flox::pkgdb::row_id> rsl = query.execute( db.db );
    EXPECT( ( rsl.size() == 1 ) && ( 0 < rsl.at( 0 ) ) );
  }

  return true;
}


/* -------------------------------------------------------------------------- */

/* Tests `license', `allowBroken', and `allowUnfree' filtering. */
bool
test_PkgQuery1( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make a package */
  row_id linux = db.addOrGetAttrSetId(
    flox::AttrPath { "legacyPackages", "x86_64-linux" } );
  row_id desc
    = db.addOrGetDescriptionId( "A program with a friendly greeting/farewell" );
  sqlite3pp::command cmd( db.db, R"SQL(
    INSERT INTO Packages (
      parentId, attrName, name, pname, version, semver, outputs, license
    , broken, unfree, descriptionId
    ) VALUES
      ( :parentId, 'hello', 'hello-2.12.1', 'hello', '2.12.1', '2.12.1'
      , '["out"]', "GPL-3.0-or-later", FALSE, FALSE, :descriptionId
      )
    , ( :parentId, 'goodbye', 'goodbye-2.12.1', 'goodbye', '2.12.1', '2.12.1'
      , '["out"]', NULL, FALSE, TRUE, :descriptionId
      )
    , ( :parentId, 'hola', 'hola-2.12.1', 'hola', '2.12.1', '2.12.1'
      , '["out"]', "BUSL-1.1", FALSE, FALSE, :descriptionId
      )
    , ( :parentId, 'ciao', 'ciao-2.12.1', 'ciao', '2.12.1', '2.12.1'
      , '["out"]', NULL, TRUE, FALSE, :descriptionId
      )
  )SQL" );
  cmd.bind( ":parentId", static_cast<long long>( linux ) );
  cmd.bind( ":descriptionId", static_cast<long long>( desc ) );
  if ( flox::pkgdb::sql_rc rc = cmd.execute(); flox::isSQLError( rc ) )
    {
      throw flox::pkgdb::PkgDbException(
        nix::fmt( "Failed to write Packages:(%d) %s", rc, db.db.error_msg() ) );
    }
  flox::pkgdb::PkgQueryArgs qargs;
  qargs.systems = std::vector<std::string> { "x86_64-linux" };

  /* Run `allowBroken = false' query */
  {
    flox::pkgdb::PkgQuery qry( qargs );
    EXPECT_EQ( qry.execute( db.db ).size(), std::size_t( 3 ) );
  }

  /* Run `allowBroken = true' query */
  {
    qargs.allowBroken = true;
    flox::pkgdb::PkgQuery qry( qargs );
    qargs.allowBroken = false;
    EXPECT_EQ( qry.execute( db.db ).size(), std::size_t( 4 ) );
  }

  /* Run `allowUnfree = true' query */
  {
    flox::pkgdb::PkgQuery qry( qargs );
    /* still omits broken */
    EXPECT_EQ( qry.execute( db.db ).size(), std::size_t( 3 ) );
  }

  /* Run `allowUnfree = false' query */
  {
    qargs.allowUnfree = false;
    flox::pkgdb::PkgQuery qry( qargs );
    qargs.allowUnfree = true;
    /* still omits broken as well */
    EXPECT_EQ( qry.execute( db.db ).size(), std::size_t( 2 ) );
  }

  /* Run `licenses = ["GPL-3.0-or-later", "BUSL-1.1", "MIT"]' query */
  {
    qargs.licenses
      = std::vector<std::string> { "GPL-3.0-or-later", "BUSL-1.1", "MIT" };
    flox::pkgdb::PkgQuery qry( qargs );
    qargs.licenses = std::nullopt;
    /* omits NULL licenses */
    EXPECT_EQ( qry.execute( db.db ).size(), std::size_t( 2 ) );
  }

  /* Run `licenses = ["BUSL-1.1", "MIT"]' query */
  {
    qargs.licenses = std::vector<std::string> { "BUSL-1.1", "MIT" };
    flox::pkgdb::PkgQuery qry( qargs );
    qargs.licenses = std::nullopt;
    /* omits NULL licenses */
    EXPECT_EQ( qry.execute( db.db ).size(), std::size_t( 1 ) );
  }

  return true;
}


/* -------------------------------------------------------------------------- */

/* Tests `partialMatch' and `pnameOrAttrName' filtering. */
bool
test_PkgQuery2( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make a package */
  row_id linux = db.addOrGetAttrSetId(
    flox::AttrPath { "legacyPackages", "x86_64-linux" } );
  row_id descGreet
    = db.addOrGetDescriptionId( "A program with a friendly hello" );
  row_id descFarewell
    = db.addOrGetDescriptionId( "A program with a friendly farewell" );
  row_id descSpecial = db.addOrGetDescriptionId(
    "A program with %%too%% 'many' [special] *chars*" );
  sqlite3pp::command cmd( db.db, R"SQL(
    INSERT INTO Packages (
      parentId, attrName, name, pname, outputs, descriptionId
    ) VALUES
      ( :parentId, 'pkg0', 'hello-2.12.1', 'hello', '["out"]', :descGreetId
      )
    , ( :parentId, 'pkg1', 'woofoo_2.12.1', 'woofoo_[*]', '["out"]', :descSpecialId
      )
    , ( :parentId, 'pkg2', 'goodbye-2.12.1', 'goodbye'
      , '["out"]', :descFarewellId
      )
    , ( :parentId, 'pkg3', 'hola-2.12.1', 'hola', '["out"]', :descGreetId
      )
    , ( :parentId, 'pkg4', 'ciao-2.12.1', 'ciao', '["out"]', :descFarewellId
      )
  )SQL" );
  cmd.bind( ":parentId", static_cast<long long>( linux ) );
  cmd.bind( ":descGreetId", static_cast<long long>( descGreet ) );
  cmd.bind( ":descFarewellId", static_cast<long long>( descFarewell ) );
  cmd.bind( ":descSpecialId", static_cast<long long>( descSpecial ) );
  if ( flox::pkgdb::sql_rc rc = cmd.execute(); flox::isSQLError( rc ) )
    {
      throw flox::pkgdb::PkgDbException(
        nix::fmt( "Failed to write Packages:(%d) %s", rc, db.db.error_msg() ) );
    }
  flox::pkgdb::PkgQueryArgs qargs;
  qargs.systems = std::vector<std::string> { "x86_64-linux" };

  // lambda to perform a search and check match results
  // exMatches is a vect of bools of the expected state of
  //    matchExactPname, matchPartialPname, matchPartialDescription respectively
  auto matchTest = [&qargs, &db]( std::string                    matchString,
                                  std::vector<std::vector<bool>> exMatches )
  {
    qargs.partialMatch = matchString;
    flox::pkgdb::PkgQuery qry(
      qargs,
      std::vector<std::string> { "matchExactPname",
                                 "matchPartialPname",
                                 "matchPartialDescription" } );
    qargs.partialMatch = std::nullopt;
    size_t count       = 0;
    auto   bound       = qry.bind( db.db );
    for ( const auto & row : *bound )
      {
        EXPECT( exMatches.size() > count );
        for ( int i : { 0, 1, 2 } )
          {
            EXPECT_EQ( row.get<bool>( i ), exMatches[count][i] );
          }
        ++count;
      }
    EXPECT_EQ( count, std::size_t( exMatches.size() ) );
    return true;
  };

  EXPECT( matchTest( "farewell", { { 0, 0, 1 }, { 0, 0, 1 } } ) );
  EXPECT( matchTest( "hel", { { 0, 1, 1 }, { 0, 0, 1 } } ) );
  EXPECT( matchTest( "hello", { { 1, 1, 1 }, { 0, 0, 1 } } ) );
  EXPECT( matchTest( "hell_", {} ) );
  EXPECT( matchTest( "hell%", {} ) );
  EXPECT( matchTest( "woofoo_[*]", { { 1, 1, 0 } } ) );
  EXPECT( matchTest( "woofoo_[*", { { 0, 1, 0 } } ) );
  EXPECT( matchTest( "woofoo_", { { 0, 1, 0 } } ) );
  EXPECT( matchTest( "'many", { { 0, 0, 1 } } ) );
  EXPECT( matchTest( "ial] *ch", { { 0, 0, 1 } } ) );
  EXPECT( matchTest( "%%too", { { 0, 0, 1 } } ) );

  // = db.addOrGetDescriptionId( "A program with \%too\% 'many' [special]
  // *chars*" ); ( :parentId, 'pkg0', 'woofoo-2.12.1', 'foo[*]', '["out"]',
  // :descSpecialId

  /* Run `pnameOrAttrName = "hello"' query, which matches pname */
  {
    qargs.pnameOrAttrName = "hello";
    flox::pkgdb::PkgQuery qry(
      qargs,
      std::vector<std::string> { "exactPname", "exactAttrName" } );
    qargs.pnameOrAttrName = std::nullopt;
    size_t count          = 0;
    auto   bound          = qry.bind( db.db );
    for ( const auto & row : *bound )
      {
        ++count;
        // exactPname is true
        EXPECT( row.get<bool>( 0 ) );
        // exactAttrName is false
        EXPECT( ! row.get<bool>( 1 ) );
      }
    EXPECT_EQ( count, std::size_t( 1 ) );
  }

  /* Run `pnameOrAttrName = "hel"' query */
  {
    qargs.pnameOrAttrName = "hel";
    flox::pkgdb::PkgQuery qry( qargs );
    qargs.pnameOrAttrName = std::nullopt;
    EXPECT( qry.execute( db.db ).empty() );
  }

  /* Run `pnameOrAttrName = "pkg0"' query, which matches attrName */
  {
    qargs.pnameOrAttrName = "pkg0";
    flox::pkgdb::PkgQuery qry(
      qargs,
      std::vector<std::string> { "exactPname", "exactAttrName" } );
    qargs.pnameOrAttrName = std::nullopt;
    size_t count          = 0;
    auto   bound          = qry.bind( db.db );
    for ( const auto & row : *bound )
      {
        ++count;
        // exactPname is false
        EXPECT( ! row.get<bool>( 0 ) );
        // exactAttrName is true
        EXPECT( row.get<bool>( 1 ) );
      }
    EXPECT_EQ( count, std::size_t( 1 ) );
  }

  return true;
}


/* -------------------------------------------------------------------------- */

/* Tests `getPackages', particularly `semver' filtering. */
bool
test_getPackages0( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make a package */
  row_id linux = db.addOrGetAttrSetId(
    flox::AttrPath { "legacyPackages", "x86_64-linux" } );
  row_id desc
    = db.addOrGetDescriptionId( "A program with a friendly greeting/farewell" );
  sqlite3pp::command cmd( db.db, R"SQL(
    INSERT INTO Packages (
      parentId, attrName, name, pname, version, semver, outputs, descriptionId
    ) VALUES
      ( :parentId, 'hello0', 'hello-2.12', 'hello', '2.12', '2.12.0'
      , '["out"]', :descriptionId
      )
    , ( :parentId, 'hello1', 'hello-2.12.1', 'hello', '2.12.1', '2.12.1'
      , '["out"]', :descriptionId
      )
    , ( :parentId, 'hello2', 'hello-3', 'hello', '3', '3.0.0'
      , '["out"]', :descriptionId
      )
  )SQL" );
  cmd.bind( ":parentId", static_cast<long long>( linux ) );
  cmd.bind( ":descriptionId", static_cast<long long>( desc ) );
  if ( flox::pkgdb::sql_rc rc = cmd.execute(); flox::isSQLError( rc ) )
    {
      throw flox::pkgdb::PkgDbException(
        nix::fmt( "Failed to write Packages:(%d) %s", rc, db.db.error_msg() ) );
    }

  flox::pkgdb::PkgQueryArgs qargs;
  qargs.systems = std::vector<std::string> { "x86_64-linux" };

  /* Run `semver = "^2"' query */
  {
    qargs.semver = { "^2" };
    size_t count = db.getPackages( qargs ).size();
    qargs.semver = std::nullopt;
    EXPECT_EQ( count, std::size_t( 2 ) );
  }

  /* Run `semver = "^3"' query */
  {
    qargs.semver = { "^3" };
    size_t count = db.getPackages( qargs ).size();
    qargs.semver = std::nullopt;
    EXPECT_EQ( count, std::size_t( 1 ) );
  }

  /* Run `semver = "^2.13"' query */
  {
    qargs.semver = { "^2.13" };
    size_t count = db.getPackages( qargs ).size();
    qargs.semver = std::nullopt;
    EXPECT_EQ( count, std::size_t( 0 ) );
  }

  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * Tests `getPackages', particularly subtree`, and
 * `system` ordering. */
bool
test_getPackages1( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make a package */
  row_id packagesLinux
    = db.addOrGetAttrSetId( flox::AttrPath { "packages", "x86_64-linux" } );
  row_id legacyDarwin = db.addOrGetAttrSetId(
    flox::AttrPath { "legacyPackages", "x86_64-darwin" } );
  row_id packagesDarwin
    = db.addOrGetAttrSetId( flox::AttrPath { "packages", "x86_64-darwin" } );

  row_id desc
    = db.addOrGetDescriptionId( "A program with a friendly greeting/farewell" );

  sqlite3pp::command cmd( db.db, R"SQL(
    INSERT INTO Packages (
      id, parentId, attrName, name, outputs, descriptionId
    ) VALUES
      ( 1, :packagesLinuxId,  'hello', 'hello', '["out"]', :descriptionId )
    , ( 2, :legacyDarwinId,   'hello', 'hello', '["out"]', :descriptionId )
    , ( 3, :packagesDarwinId, 'hello', 'hello', '["out"]', :descriptionId )
  )SQL" );
  cmd.bind( ":descriptionId", static_cast<long long>( desc ) );
  cmd.bind( ":packagesLinuxId", static_cast<long long>( packagesLinux ) );
  cmd.bind( ":legacyDarwinId", static_cast<long long>( legacyDarwin ) );
  cmd.bind( ":packagesDarwinId", static_cast<long long>( packagesDarwin ) );
  if ( flox::pkgdb::sql_rc rc = cmd.execute(); flox::isSQLError( rc ) )
    {
      throw flox::pkgdb::PkgDbException(
        nix::fmt( "Failed to write Packages:(%d) %s", rc, db.db.error_msg() ) );
    }

  flox::pkgdb::PkgQueryArgs qargs;
  qargs.systems = std::vector<std::string> {};

  /* Test `subtrees` ordering */
  {
    qargs.systems = std::vector<std::string> { "x86_64-darwin" };
    qargs.subtrees
      = std::vector<flox::Subtree> { flox::ST_PACKAGES, flox::ST_LEGACY };
    EXPECT( db.getPackages( qargs ) == ( std::vector<row_id> { 3, 2 } ) );
    qargs.subtrees
      = std::vector<flox::Subtree> { flox::ST_LEGACY, flox::ST_PACKAGES };
    EXPECT( db.getPackages( qargs ) == ( std::vector<row_id> { 2, 3 } ) );
    qargs.subtrees = std::nullopt;
    qargs.systems  = std::vector<std::string> {};
  }

  /* Test `systems` ordering */
  {
    qargs.subtrees = std::vector<flox::Subtree> { flox::ST_PACKAGES };
    qargs.systems
      = std::vector<std::string> { "x86_64-linux", "x86_64-darwin" };
    EXPECT( db.getPackages( qargs ) == ( std::vector<row_id> { 1, 3 } ) );
    qargs.systems
      = std::vector<std::string> { "x86_64-darwin", "x86_64-linux" };
    EXPECT( db.getPackages( qargs ) == ( std::vector<row_id> { 3, 1 } ) );
    qargs.systems  = std::vector<std::string> {};
    qargs.subtrees = std::nullopt;
  }

  return true;
}


/* -------------------------------------------------------------------------- */

/** Tests `getPackages', particularly `version' ordering. */
bool
test_getPackages2( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make a package */
  row_id linux
    = db.addOrGetAttrSetId( flox::AttrPath { "packages", "x86_64-linux" } );

  sqlite3pp::command cmd( db.db, R"SQL(
    INSERT INTO Packages (
      id, parentId, attrName, name, pname, version, semver, outputs
    ) VALUES
      ( 1, :parentId, 'hello0', 'hello-2.12.0', 'hello', '2.12.0', '2.12.0'
      , '["out"]' )
    , ( 2, :parentId, 'hello1', 'hello-2.12.1-pre', 'hello', '2.12.1-pre'
      , '2.12.1-pre', '["out"]' )
    , ( 3, :parentId, 'hello2', 'hello-2.13', 'hello', '2.13', '2.13.0'
      , '["out"]' )
    , ( 4, :parentId, 'hello3', 'hello', 'hello', NULL, NULL, '["out"]' )
    , ( 5, :parentId, 'hello4', 'hello-1917-10-26', 'hello', '1917-10-26', NULL
      , '["out"]' )
    , ( 6, :parentId, 'hello5', 'hello-1917-10-25', 'hello', '1917-10-25', NULL
      , '["out"]' )
    , ( 7, :parentId, 'hello6', 'hello-junk', 'hello', 'junk', NULL, '["out"]' )
    , ( 8, :parentId, 'hello7', 'hello-trunk', 'hello', 'trunk', NULL
      , '["out"]' )
  )SQL" );
  cmd.bind( ":parentId", static_cast<long long>( linux ) );
  if ( flox::pkgdb::sql_rc rc = cmd.execute(); flox::isSQLError( rc ) )
    {
      throw flox::pkgdb::PkgDbException(
        nix::fmt( "Failed to write Packages:(%d) %s", rc, db.db.error_msg() ) );
    }

  flox::pkgdb::PkgQueryArgs qargs;
  qargs.subtrees = std::vector<flox::Subtree> { flox::ST_PACKAGES };
  qargs.systems  = std::vector<std::string> { "x86_64-linux" };

  /* Test `preferPreReleases = false' ordering */
  qargs.preferPreReleases = false;
  EXPECT( db.getPackages( qargs )
          == ( std::vector<row_id> { 3, 1, 2, 5, 6, 7, 8, 4 } ) );

  qargs.preferPreReleases = true;
  /* Test `preferPreReleases = true' ordering */
  EXPECT( db.getPackages( qargs )
          == ( std::vector<row_id> { 3, 2, 1, 5, 6, 7, 8, 4 } ) );

  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_DbPackage0( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make a package */
  row_id linux = db.addOrGetAttrSetId(
    flox::AttrPath { "legacyPackages", "x86_64-linux" } );
  row_id desc
    = db.addOrGetDescriptionId( "A program with a friendly greeting/farewell" );
  sqlite3pp::command cmd( db.db, R"SQL(
    INSERT INTO Packages (
      parentId, attrName, name, pname, version, semver, license, outputs
    , outputsToInstall, broken, unfree, descriptionId
    ) VALUES
      ( :parentId, 'hello', 'hello-2.12', 'hello', '2.12', '2.12.0'
      , 'GPL-3.0-or-later', '["out"]', '["out"]', false, false, :descriptionId
      )
  )SQL" );
  cmd.bind( ":parentId", static_cast<long long>( linux ) );
  cmd.bind( ":descriptionId", static_cast<long long>( desc ) );
  if ( flox::pkgdb::sql_rc rc = cmd.execute(); flox::isSQLError( rc ) )
    {
      throw flox::pkgdb::PkgDbException(
        nix::fmt( "Failed to write Packages:(%d) %s", rc, db.db.error_msg() ) );
    }
  row_id pkgId = db.db.last_insert_rowid();
  auto   pkg
    = flox::pkgdb::DbPackage( static_cast<flox::pkgdb::PkgDbReadOnly &>( db ),
                              pkgId );

  EXPECT( pkg.getPathStrs()
          == ( flox::AttrPath { "legacyPackages", "x86_64-linux", "hello" } ) );
  EXPECT_EQ( pkg.getFullName(), "hello-2.12" );
  EXPECT_EQ( pkg.getPname(), "hello" );
  EXPECT_EQ( *pkg.getVersion(), "2.12" );
  EXPECT_EQ( *pkg.getSemver(), "2.12.0" );
  EXPECT_EQ( *pkg.getLicense(), "GPL-3.0-or-later" );
  EXPECT( pkg.getOutputs() == ( std::vector<std::string> { "out" } ) );
  EXPECT( pkg.getOutputsToInstall() == ( std::vector<std::string> { "out" } ) );
  EXPECT_EQ( *pkg.isBroken(), false );
  EXPECT_EQ( *pkg.isUnfree(), false );
  EXPECT_EQ( *pkg.getDescription(),
             "A program with a friendly greeting/farewell" );
  EXPECT_EQ( pkgId, pkg.getPackageId() );
  EXPECT_EQ( pkg.getDbPath(), db.dbPath );
  EXPECT_EQ( nix::parseFlakeRef( nixpkgsRef ).to_string(),
             pkg.getLockedFlakeRef().to_string() );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_getPackages_semver0( flox::pkgdb::PkgDb & db )
{
  clearTables( db );

  /* Make packages */
  row_id linux = db.addOrGetAttrSetId(
    flox::AttrPath { "legacyPackages", "x86_64-linux" } );
  row_id desc
    = db.addOrGetDescriptionId( "A program with a friendly greeting/farewell" );
  sqlite3pp::command cmd( db.db, R"SQL(
    INSERT INTO Packages (
      parentId, attrName, name, pname, version, semver, license, outputs
    , outputsToInstall, broken, unfree, descriptionId
    ) VALUES
      ( :parentId, 'hello0', 'hello-2.12', 'hello', '2.12', '2.12.0'
      , 'GPL-3.0-or-later', '["out"]', '["out"]', false, false, :descriptionId
      )
    , ( :parentId, 'hello1', 'hello-2.13.1', 'hello', '2.13.1', '2.13.1'
      , 'GPL-3.0-or-later', '["out"]', '["out"]', false, false, :descriptionId
      )
    , ( :parentId, 'hello2', 'hello-2.14.1', 'hello', '2.14.1', '2.14.1'
      , 'GPL-3.0-or-later', '["out"]', '["out"]', false, false, :descriptionId
      )
    , ( :parentId, 'hello3', 'hello-3', 'hello', '3', '3.0.0'
      , 'GPL-3.0-or-later', '["out"]', '["out"]', false, false, :descriptionId
      )
    , ( :parentId, 'hello4', 'hello-4.2.0', 'hello', '4.2', '4.2.0'
      , 'GPL-3.0-or-later', '["out"]', '["out"]', false, false, :descriptionId
      )
    , ( :parentId, 'hello5', 'hello-no-version', 'hello', NULL, NULL
      , 'GPL-3.0-or-later', '["out"]', '["out"]', false, false, :descriptionId
      )
  )SQL" );
  cmd.bind( ":parentId", static_cast<long long>( linux ) );
  cmd.bind( ":descriptionId", static_cast<long long>( desc ) );
  if ( flox::pkgdb::sql_rc rc = cmd.execute(); flox::isSQLError( rc ) )
    {
      throw flox::pkgdb::PkgDbException(
        nix::fmt( "Failed to write Packages:(%d) %s", rc, db.db.error_msg() ) );
    }

  flox::pkgdb::PkgQueryArgs qargs;
  qargs.subtrees = std::vector<flox::Subtree> { flox::ST_LEGACY };
  qargs.systems  = std::vector<std::string> { "x86_64-linux" };
  qargs.pname    = "hello";

  auto getSemvers =
    [&]( const std::string & semver ) -> std::vector<std::optional<std::string>>
  {
    std::vector<std::optional<std::string>> rsl;
    qargs.semver = { semver };
    for ( flox::pkgdb::row_id rowId : db.getPackages( qargs ) )
      {
        rsl.emplace_back( flox::pkgdb::DbPackage(
                            static_cast<flox::pkgdb::PkgDbReadOnly &>( db ),
                            rowId )
                            .getSemver() );
      }
    return rsl;
  };

  /* ^2 : 2.0.0 <= VERSION < 3.0.0 */
  {
    auto semvers = getSemvers( "^2" );
    EXPECT_EQ( semvers.size(), std::size_t( 3 ) );
    size_t idx = 0;
    for ( const std::optional<std::string> & maybeSemver : semvers )
      {
        EXPECT( maybeSemver.has_value() );
        if ( idx == 0 ) { EXPECT_EQ( *maybeSemver, "2.14.1" ); }
        else if ( idx == 1 ) { EXPECT_EQ( *maybeSemver, "2.13.1" ); }
        else if ( idx == 2 ) { EXPECT_EQ( *maybeSemver, "2.12.0" ); }
        ++idx;
      }
  }

  /* ^2.13.1 : 2.13.1 <= VERSION < 3.0.0 */
  {
    auto semvers = getSemvers( "^2.13.1" );
    EXPECT_EQ( semvers.size(), std::size_t( 2 ) );
    size_t idx = 0;
    for ( const std::optional<std::string> & maybeSemver : semvers )
      {
        EXPECT( maybeSemver.has_value() );
        if ( idx == 0 ) { EXPECT_EQ( *maybeSemver, "2.14.1" ); }
        else if ( idx == 1 ) { EXPECT_EQ( *maybeSemver, "2.13.1" ); }
        ++idx;
      }
  }

  /* '*' : Any semantic version, should omit `hello-no-version' */
  {
    auto semvers = getSemvers( "*" );
    EXPECT_EQ( semvers.size(), std::size_t( 5 ) );
    for ( const auto & maybeSemver : semvers )
      {
        EXPECT( maybeSemver.has_value() );
      }
  }

  return true;
}

/* -------------------------------------------------------------------------- */

/**
 * @brief Ensure parsing @a flox::pkgdb::RulesTreeNode from JSON doesn't throw.
 */
bool
test_RulesTree_parse0()
{
  flox::pkgdb::ScrapeRules rules( rulesJSON );
  return true;
}

/**
 * @brief Ensure the hash is generated for the rules and is deterministic.
 */
bool
test_RulesTree_hash()
{
  flox::pkgdb::ScrapeRules rules( rulesJSON );
  auto                     hashStr = rules.hashString();
  EXPECT_EQ( hashStr, "md5:9d81a5b907db9b19cc84ba41af36150b" );
  return true;
}

bool
test_RulesTree_parse0_badRules()
{
  const nlohmann::json badRulesJSON = R"( {
    "allowRecursive": [
      ["legacyPackages", null, "darwin"],
      ["legacyPackages", null, null, "darwin"]
    ]
  } )"_json;

  try
    {
      flox::pkgdb::RulesTreeNode rules(
        badRulesJSON.get<flox::pkgdb::ScrapeRulesRaw>() );
    }
  catch ( const flox::FloxException & err )
    {
      /* expect this to be thown on account of a bad rule. */
      return true;
    }
  return false;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Ensure parsing @a flox::pkgdb::RulesTreeNode from JSON sets the
 *        expected fields.
 */
bool
test_RulesTree_parse1()
{
  flox::pkgdb::ScrapeRules     scrapeRules( rulesJSON );
  flox::pkgdb::RulesTreeNode   rulesRoot = scrapeRules.getRootNode();
  flox::pkgdb::RulesTreeNode * node      = &rulesRoot;
  flox::AttrPath path = { "legacyPackages", "x86_64-linux", "darwin" };
  for ( const std::string & attr : path )
    {
      if ( node->children.find( attr ) == node->children.end() )
        {
          EXPECT_FAIL( "RulesTreeNode missing child `" + attr + '\'' );
        }
      node = &node->children.at( attr );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Ensure that a path without a rule uses the _default_ rule. */
bool
test_RulesTree_getRule_fallback0()
{
  flox::pkgdb::ScrapeRules   scrapeRules( rulesJSON );
  flox::pkgdb::RulesTreeNode rulesRoot = scrapeRules.getRootNode();
  flox::pkgdb::ScrapeRule    rule
    = rulesRoot.getRule( flox::AttrPath { "phony" } );
  EXPECT_EQ( rule, flox::pkgdb::SR_DEFAULT );
  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Ensure `null` glob works for all systems. */
bool
test_RulesTree_getRule0()
{
  flox::pkgdb::ScrapeRules   scrapeRules( rulesJSON );
  flox::pkgdb::RulesTreeNode rulesRoot = scrapeRules.getRootNode();

  flox::pkgdb::ScrapeRule rule
    = rulesRoot.getRule( flox::AttrPath { "legacyPackages", "x86_64-linux" } );
  EXPECT_EQ( rule, flox::pkgdb::SR_DEFAULT );

  rule = rulesRoot.getRule(
    flox::AttrPath { "legacyPackages", "x86_64-linux", "darwin" } );
  EXPECT_EQ( rule, flox::pkgdb::SR_ALLOW_RECURSIVE );

  rule = rulesRoot.getRule(
    flox::AttrPath { "legacyPackages", "x86_64-darwin", "darwin" } );
  EXPECT_EQ( rule, flox::pkgdb::SR_ALLOW_RECURSIVE );

  rule = rulesRoot.getRule(
    flox::AttrPath { "legacyPackages", "aarch64-linux", "darwin" } );
  EXPECT_EQ( rule, flox::pkgdb::SR_ALLOW_RECURSIVE );

  rule = rulesRoot.getRule(
    flox::AttrPath { "legacyPackages", "aarch64-darwin", "darwin" } );
  EXPECT_EQ( rule, flox::pkgdb::SR_ALLOW_RECURSIVE );

  rule = rulesRoot.getRule( flox::AttrPath { "legacyPackages",
                                             "aarch64-darwin",
                                             "swiftPackages",
                                             "darwin" } );
  EXPECT_EQ( rule, flox::pkgdb::SR_ALLOW_RECURSIVE );
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Ensure nested `allowPackage` under `disallowRecursive` is preserved.
 */
bool
test_RulesTree_getRule1()
{
  flox::pkgdb::ScrapeRules   scrapeRules( rulesJSON );
  flox::pkgdb::RulesTreeNode rulesRoot = scrapeRules.getRootNode();

  flox::pkgdb::ScrapeRule rule = rulesRoot.children.at( "legacyPackages" )
                                   .children.at( "x86_64-linux" )
                                   .children.at( "python310Packages" )
                                   .children.at( "pip" )
                                   .rule;
  EXPECT_EQ( rule, flox::pkgdb::SR_ALLOW_PACKAGE );

  flox::pkgdb::ScrapeRule rule2
    = rulesRoot.getRule( flox::AttrPath { "legacyPackages",
                                          "x86_64-linux",
                                          "python310Packages",
                                          "pip" } );
  EXPECT_EQ( rule2, flox::pkgdb::SR_ALLOW_PACKAGE );
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 *  @brief Ensure @a flox::pkgdb::RulesTreeNode::getRule() does not _inherit_
 *         parent rules.
 *
 * Inheritance is the responsibility of
 * @a flox::pkgdb::RulesTreeNode::applyRules(), while `getRule()` should return
 * the *exact* value of @a rule at an attribute path.
 */
bool
test_RulesTree_getRule2()
{
  flox::pkgdb::ScrapeRules   scrapeRules( rulesJSON );
  flox::pkgdb::RulesTreeNode rulesRoot = scrapeRules.getRootNode();

  flox::pkgdb::ScrapeRule rule = rulesRoot.children.at( "legacyPackages" )
                                   .children.at( "x86_64-linux" )
                                   .children.at( "python310Packages" )
                                   .children.at( "pip" )
                                   .rule;
  EXPECT_EQ( rule, flox::pkgdb::SR_ALLOW_PACKAGE );

  flox::pkgdb::ScrapeRule rule2
    = rulesRoot.getRule( flox::AttrPath { "legacyPackages",
                                          "x86_64-linux",
                                          "python310Packages",
                                          "pip" } );
  EXPECT_EQ( rule2, flox::pkgdb::SR_ALLOW_PACKAGE );

  flox::pkgdb::ScrapeRule rule3
    = rulesRoot.getRule( flox::AttrPath { "legacyPackages",
                                          "x86_64-linux",
                                          "swiftPackages",
                                          "darwin" } );
  EXPECT_EQ( rule3, flox::pkgdb::SR_ALLOW_RECURSIVE );
  return true;
}

bool
test_scrapeMemoryUse()
{
  using flox::pkgdb::PkgDbInput;
  const char * envVar              = "FLOX_AVAILABLE_MEMORY";
  const char * existingMemOverride = getenv( envVar );

  // Using discovered 'available memory' shall be within the min and max
  // defined.
  size_t pageSize = PkgDbInput::getScrapingPageSize();
  EXPECT( pageSize >= PkgDbInput::minPageSize
          && pageSize <= PkgDbInput::maxPageSize );

  // Limit to lower bound for 1GB availble memory
  setenv( envVar, std::to_string( 1 * 1024 * 1024 ).c_str(), 1 );
  EXPECT( PkgDbInput::getScrapingPageSize() == PkgDbInput::minPageSize );

  // Limit to upper bound for 8GB available memory
  setenv( envVar, std::to_string( 8 * 1024 * 1024 ).c_str(), 1 );
  EXPECT( PkgDbInput::getScrapingPageSize() == PkgDbInput::maxPageSize );

  // Clear this out for the remainder of the process
  if ( existingMemOverride ) { setenv( envVar, existingMemOverride, 1 ); }
  else { unsetenv( envVar ); }
  return true;
}

/* -------------------------------------------------------------------------- */

int
main( int argc, char * argv[] )
{
  int ec = EXIT_SUCCESS;
#define RUN_TEST( ... ) _RUN_TEST( ec, __VA_ARGS__ )

  nix::verbosity = nix::lvlWarn;
  if ( ( 1 < argc ) && ( std::string_view( argv[1] ) == "-v" ) )
    {
      nix::verbosity = nix::lvlDebug;
    }
  else if ( ( 1 < argc ) && ( std::string_view( argv[1] ) == "-vv" ) )
    {
      nix::verbosity = nix::lvlVomit;
    }
  /* Initialize `nix' */
  flox::NixState nstate;


  auto [fd, path] = nix::createTempFile( "test-pkgdb.sql" );
  fd.close();

  nix::FlakeRef ref = nix::parseFlakeRef( nixpkgsRef );

  flox::FloxFlake flake( nstate.getState(), ref );

  {
    flox::pkgdb::PkgDb db( flake.lockedFlake, path );

    RUN_TEST( addOrGetAttrSetId0, db );
    RUN_TEST( addOrGetAttrSetId1, db );

    RUN_TEST( getDbVersion0, db );

    RUN_TEST( hasAttrSet0, db );
    RUN_TEST( hasAttrSet1, db );

    RUN_TEST( getAttrSetId0, db );

    RUN_TEST( getAttrSetPath0, db );

    RUN_TEST( hasPackage0, db );

    RUN_TEST( descriptions0, db );

    RUN_TEST( PkgQuery0, db );
    RUN_TEST( PkgQuery1, db );
    RUN_TEST( PkgQuery2, db );

    RUN_TEST( getPackages0, db );
    RUN_TEST( getPackages1, db );
    RUN_TEST( getPackages2, db );

    RUN_TEST( DbPackage0, db );

    RUN_TEST( getPackages_semver0, db );

    RUN_TEST( scrapeMemoryUse );

    RUN_TEST( RulesTree_parse0 );
    RUN_TEST( RulesTree_parse0_badRules );
    RUN_TEST( RulesTree_parse1 );
    RUN_TEST( RulesTree_getRule_fallback0 );
    RUN_TEST( RulesTree_getRule0 );
    RUN_TEST( RulesTree_getRule1 );
    RUN_TEST( RulesTree_getRule2 );
    RUN_TEST( RulesTree_hash );
  }

  /* XXX: You may find it useful to preserve the file and print it for some
   *      debugging efforts. */
  std::filesystem::remove( path );
  // std::cerr << path << std::endl;

  return ec;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
