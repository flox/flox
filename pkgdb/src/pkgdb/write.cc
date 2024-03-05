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
#include <ranges>
#include <string>

#include <nlohmann/json.hpp>

#include "flox/core/util.hh"
#include "flox/flake-package.hh"
#include "flox/pkgdb/write.hh"

#include "./schemas.hh"

/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

std::string
scrapeRuleToString( ScrapeRule rule )
{
  switch ( rule )
    {
      case SR_DEFAULT: return "default";
      case SR_ALLOW_PACKAGE: return "allowPackage";
      case SR_DISALLOW_PACKAGE: return "disallowPackage";
      case SR_ALLOW_RECURSIVE: return "allowRecursive";
      case SR_DISALLOW_RECURSIVE: return "disallowRecursive";
      default: return "UNKNOWN";
    }
}


/* -------------------------------------------------------------------------- */

void
RulesTreeNode::addRule( AttrPathGlob & relPath, ScrapeRule rule )
{
  /* Modify our rule. */
  if ( relPath.empty() )
    {
      if ( this->rule != SR_DEFAULT )
        {
          // TODO: Pass abs-path
          throw FloxException( "attempted to overwrite existing rule '"
                               + scrapeRuleToString( this->rule ) + "' for '"
                               + this->attrName + "' with new rule '"
                               + scrapeRuleToString( rule ) + "'" );
        }
      traceLog( "assigning rule to '" + scrapeRuleToString( rule ) + "' to '"
                + this->attrName + '\'' );
      this->rule = rule;
      return;
    }
  traceLog( "adding rule to '" + this->attrName + "': '"
            + displayableGlobbedPath( relPath ) + " = "
            + scrapeRuleToString( rule ) + '\'' );

  /* Handle system glob by splitting into 4 recursive calls. */
  if ( ! relPath.front().has_value() )
    {
      // TODO: This does not allow scraping of "packages" trees, and it allows
      // for
      //      "legacyPackages.legacyPackages.*.**".  Both should probably be
      //      addressed but do not pose a functional threat at the moment.
      if ( this->attrName != "legacyPackages" )
        {
          throw FloxException(
            "glob in rules (null) only allowed as child of legacyPackages" );
        }

      traceLog( "splitting system glob into real systems" );
      for ( const auto & system : getDefaultSystems() )
        {
          AttrPathGlob relPathCopy = relPath;
          relPathCopy.front()      = system;
          this->addRule( relPathCopy, rule );
        }
      return;
    }

  std::string attrName = std::move( *relPath.front() );
  // TODO: Use a `std::deque' instead of `std::vector' for efficiency.
  //       This container is only used for `push_back' and `pop_front'.
  //       Removing from the front is inefficient for `std::vector'.
  relPath.erase( relPath.begin() );

  if ( auto it = this->children.find( attrName ); it != this->children.end() )
    {
      traceLog( "found existing child '" + attrName + '\'' );
      /* Add to existing child node. */
      it->second.addRule( relPath, rule );
    }
  else if ( relPath.empty() )
    {
      /* Add leaf node. */
      traceLog( "creating leaf '" + attrName + " = "
                + scrapeRuleToString( rule ) + '\'' );
      this->children.emplace( attrName, RulesTreeNode( attrName, rule ) );
    }
  else
    {
      traceLog( "creating child '" + attrName + '\'' );
      /* Create a new child node. */
      this->children.emplace( attrName, RulesTreeNode( attrName ) );
      this->children.at( attrName ).addRule( relPath, rule );
    }
}


/* -------------------------------------------------------------------------- */

ScrapeRule
RulesTreeNode::getRule( const AttrPath & path ) const
{
  const RulesTreeNode * node = this;
  for ( const auto & attrName : path )
    {
      try
        {
          node = &node->children.at( attrName );
        }
      catch ( const std::out_of_range & err )
        {
          return SR_DEFAULT;
        }
    }
  return node->rule;
}


/* -------------------------------------------------------------------------- */

std::optional<bool>
RulesTreeNode::applyRules( const AttrPath & path ) const
{
  auto rule = this->getRule( path );
  /* Perform lookup in parents if necessary. */
  if ( rule == SR_DEFAULT )
    {
      AttrPath pathCopy = path;
      do {
          pathCopy.pop_back();
          rule = this->getRule( pathCopy );
        }
      while ( ( rule == SR_DEFAULT ) && ( ! pathCopy.empty() ) );
    }

  switch ( rule )
    {
      case SR_ALLOW_PACKAGE: return true;
      case SR_DISALLOW_PACKAGE: return false;
      case SR_ALLOW_RECURSIVE: return true;
      case SR_DISALLOW_RECURSIVE: return false;
      case SR_DEFAULT: return std::nullopt;
      default:
        throw PkgDbException( "encountered unexpected rule '"
                              + scrapeRuleToString( rule ) + '\'' );
    }
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, RulesTreeNode & rules )
{
  ScrapeRulesRaw raw = jfrom;
  rules              = static_cast<RulesTreeNode>( raw );
}


/* -------------------------------------------------------------------------- */

void
to_json( nlohmann::json & jto, const RulesTreeNode & rules )
{
  jto = { { "__rule", scrapeRuleToString( rules.rule ) } };
  for ( const auto & [name, child] : rules.children )
    {
      nlohmann::json jchild;
      to_json( jchild, child );
      jto[name] = jchild;
    }
}


/* -------------------------------------------------------------------------- */

RulesTreeNode::RulesTreeNode( ScrapeRulesRaw raw )
{
  /* Add rules in order of precedence */
  for ( const auto & path : raw.allowPackage )
    {
      AttrPathGlob pathCopy( std::move( path ) );
      this->addRule( pathCopy, SR_ALLOW_PACKAGE );
    }
  for ( const auto & path : raw.disallowPackage )
    {
      AttrPathGlob pathCopy( std::move( path ) );
      this->addRule( pathCopy, SR_DISALLOW_PACKAGE );
    }
  for ( const auto & path : raw.allowRecursive )
    {
      AttrPathGlob pathCopy( std::move( path ) );
      this->addRule( pathCopy, SR_ALLOW_RECURSIVE );
    }
  for ( const auto & path : raw.disallowRecursive )
    {
      AttrPathGlob pathCopy( std::move( path ) );
      this->addRule( pathCopy, SR_DISALLOW_RECURSIVE );
    }
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, ScrapeRulesRaw & rules )
{
  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "allowPackage" )
        {
          for ( const auto & path : value )
            {
              try
                {
                  rules.allowPackage.emplace_back( path );
                }
              catch ( nlohmann::json::exception & err )
                {
                  throw PkgDbException(
                    "couldn't interpret field `allowPackage." + key + "': ",
                    flox::extract_json_errmsg( err ) );
                }
            }
        }
      else if ( key == "disallowPackage" )
        {
          for ( const auto & path : value )
            {
              try
                {
                  rules.disallowPackage.emplace_back( path );
                }
              catch ( nlohmann::json::exception & err )
                {
                  throw PkgDbException(
                    "couldn't interpret field `disallowPackage." + key + "': ",
                    flox::extract_json_errmsg( err ) );
                }
            }
        }
      else if ( key == "allowRecursive" )
        {
          for ( const auto & path : value )
            {
              try
                {
                  rules.allowRecursive.emplace_back( path );
                }
              catch ( nlohmann::json::exception & err )
                {
                  throw PkgDbException(
                    "couldn't interpret field `allowRecursive." + key + "': ",
                    flox::extract_json_errmsg( err ) );
                }
            }
        }
      else if ( key == "disallowRecursive" )
        {
          for ( const auto & path : value )
            {
              try
                {
                  rules.disallowRecursive.emplace_back( path );
                }
              catch ( nlohmann::json::exception & err )
                {
                  throw PkgDbException(
                    "couldn't interpret field `disallowRecursive." + key
                      + "': ",
                    flox::extract_json_errmsg( err ) );
                }
            }
        }
      else { throw FloxException( "unknown scrape rule: '" + key + "'" ); }
    }
}

/* -------------------------------------------------------------------------- */

/** @brief Create views in database if they do not exist. */
static void
initViews( PkgDb & pdb )
{
  if ( sql_rc rcode = pdb.execute_all( sql_views ); isSQLError( rcode ) )
    {
      throw PkgDbException( nix::fmt( "failed to initialize views:(%d) %s",
                                      rcode,
                                      pdb.db.error_msg() ) );
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
  /* Drop all existing views. */
  {
    sqlite3pp::query qry( pdb.db,
                          "SELECT name FROM sqlite_master WHERE"
                          " ( type = 'view' )" );
    for ( auto row : qry )
      {
        auto        name = row.get<std::string>( 0 );
        std::string cmd  = "DROP VIEW IF EXISTS '" + name + '\'';
        if ( sql_rc rcode = pdb.execute( cmd.c_str() ); isSQLError( rcode ) )
          {
            throw PkgDbException( nix::fmt( "failed to drop view '%s':(%d) %s",
                                            name,
                                            rcode,
                                            pdb.db.error_msg() ) );
          }
      }
  }

  /* Update the `pkgdb_views_schema' version. */
  sqlite3pp::command updateVersion(
    pdb.db,
    "UPDATE DbVersions SET version = ? WHERE name = 'pkgdb_views_schema'" );
  updateVersion.bind( 1, static_cast<int>( sqlVersions.views ) );

  if ( sql_rc rcode = updateVersion.execute(); isSQLError( rcode ) )
    {
      throw PkgDbException( nix::fmt( "failed to update PkgDb Views:(%d) %s",
                                      rcode,
                                      pdb.db.error_msg() ) );
    }

  /* Redefine the `VIEW's */
  initViews( pdb );
}


/* -------------------------------------------------------------------------- */

/** @brief Create tables in database if they do not exist. */
static void
initTables( PkgDb & pdb )
{
  if ( sql_rc rcode = pdb.execute( sql_versions ); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to initialize DbVersions table:(%d) %s",
                  rcode,
                  pdb.db.error_msg() ) );
    }

  if ( sql_rc rcode = pdb.execute_all( sql_input ); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to initialize LockedFlake table:(%d) %s",
                  rcode,
                  pdb.db.error_msg() ) );
    }

  if ( sql_rc rcode = pdb.execute_all( sql_attrSets ); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to initialize AttrSets table:(%d) %s",
                  rcode,
                  pdb.db.error_msg() ) );
    }

  if ( sql_rc rcode = pdb.execute_all( sql_packages ); isSQLError( rcode ) )
    {
      throw PkgDbException(
        nix::fmt( "failed to initialize Packages table:(%d) %s",
                  rcode,
                  pdb.db.error_msg() ) );
    }
}


/* -------------------------------------------------------------------------- */

/** @brief Create `DbVersions` rows if they do not exist. */
static void
initVersions( PkgDb & pdb )
{
  sqlite3pp::command defineVersions(
    pdb.db,
    "INSERT OR IGNORE INTO DbVersions ( name, version ) VALUES"
    "  ( 'pkgdb',        '" FLOX_PKGDB_VERSION "' )"
    ", ( 'pkgdb_tables_schema', ? )"
    ", ( 'pkgdb_views_schema', ? )" );
  defineVersions.bind( 1, static_cast<int>( sqlVersions.tables ) );
  defineVersions.bind( 2, static_cast<int>( sqlVersions.views ) );
  if ( sql_rc rcode = defineVersions.execute(); isSQLError( rcode ) )
    {
      throw PkgDbException( "failed to write DbVersions info",
                            pdb.db.error_msg() );
    }
}


/* -------------------------------------------------------------------------- */

void
PkgDb::init()
{
  initTables( *this );
  initVersions( *this );

  /* If the views version is outdated, update them. */
  if ( this->getDbVersion().views < sqlVersions.views )
    {
      updateViews( *this );
    }
  else { initViews( *this ); }
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
  this->db.connect( this->dbPath.string().c_str(),
                    SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE );
  this->db.set_busy_timeout( DB_BUSY_TIMEOUT );
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
            nix::fmt( "failed to add AttrSet.id 'AttrSets[%ull].%s':(%d) %s",
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
  auto itr = qry.begin();
  if ( itr != qry.end() )
    {
      nix::Activity act(
        *nix::logger,
        nix::lvlDebug,
        nix::actUnknown,
        nix::fmt( "found existing description in database: %s.",
                  description ) );
      return ( *itr ).get<long long>( 0 );
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
                   const flox::Cursor & cursor )
{
  sqlite3pp::command cmd( this->db, R"SQL(
    INSERT OR REPLACE INTO Packages (
      parentId, attrName, name, pname, version, semver, license
    , outputs, outputsToInstall, broken, unfree, descriptionId
    ) VALUES (
      :parentId, :attrName, :name, :pname, :version, :semver, :license
    , :outputs, :outputsToInstall, :broken, :unfree, :descriptionId
    )
  )SQL" );

  /* We don't need to reference any `attrPath' related info here, so
   * we can avoid looking up the parent path by passing a phony one to the
   * `FlakePackage' constructor here. */
  FlakePackage pkg( cursor, { "packages", "x86_64-linux", "phony" }, true );
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

/* Currently returns the one and only set of rules for scraping.
 * These are hardcoded for now.
 * TODO: make the rules file to use a command line argument or otherwise
 * configurable.
 */
static const RulesTreeNode &
getDefaultRules()
{
  static std::optional<RulesTreeNode> rules;
  if ( ! rules.has_value() )
    {
      /* These are just hardcoded for now.*/
      std::string_view rulesJSON = (
#include "./rules.json.hh"
      );

      ScrapeRulesRaw raw = nlohmann::json::parse( rulesJSON );
      rules              = RulesTreeNode( std::move( raw ) );
    }
  return *rules;
}

void
PkgDb::processSingleAttrib( const nix::SymbolStr &    sym,
                            const flox::Cursor &      cursor,
                            const flox::AttrPath &    prefix,
                            const flox::pkgdb::row_id parentId,
                            const flox::subtree_type  subtree,
                            Todos &                   todo )
{
  auto getPathString = [&prefix, &sym]() -> const std::string
  { return concatStringsSep( ".", prefix ) + "." + sym; };

  try
    {

      flox::AttrPath path = prefix;
      path.emplace_back( sym );

      /* If the package or prefix is disallowed, bail. */
      std::optional<bool> rulesBasedOverride
        = getDefaultRules().applyRules( path );
      if ( rulesBasedOverride.has_value() && ( ! ( *rulesBasedOverride ) ) )
        {
          if ( nix::lvlTalkative <= nix::verbosity )
            {
              traceLog( "scrapeRules: skipping disallowed attribute: "
                        + getPathString() );
            }
          return;
        }

      if ( cursor->isDerivation() )
        {
          this->addPackage( parentId, sym, cursor );
        }
      else if ( subtree == ST_PACKAGES )
        {
          /* Do not recurse down the `packages` subtree */
          return;
        }
      else
        {
          bool allowed = rulesBasedOverride.has_value()
                           ? rulesBasedOverride.value()
                           : [&cursor]() -> bool
          {
            auto maybeRecurse = cursor->maybeGetAttr( "recurseForDerivations" );
            return maybeRecurse != nullptr && maybeRecurse->getBool();
          }();

          if ( nix::lvlTalkative <= nix::verbosity
               && rulesBasedOverride.has_value() )
            {
              traceLog(
                nix::fmt( "scrapeRules: matching rule found (%s), for %s\n",
                          rulesBasedOverride.value() ? "true" : "false",
                          getPathString() ) );
            }

          if ( allowed )
            {
              row_id childId = this->addOrGetAttrSetId( sym, parentId );
              todo.emplace(
                std::make_tuple( std::move( path ), cursor, childId ) );
            }
        }
    }
  catch ( const nix::EvalError & err )
    {
      /* Ignore errors in `legacyPackages' */
      if ( subtree == ST_LEGACY )
        {
          /* Only print eval errors in "debug" mode. */
          nix::ignoreException( nix::lvlDebug );
          return;
        }

      throw;
    }
}


/* -------------------------------------------------------------------------- */

/* NOTE:
 * Benchmarks on large catalogs have indicated that using a _todo_ queue instead
 * of recursion is faster and consumes less memory.
 * Repeated runs against `nixpkgs-flox` come in at ~2m03s using recursion and
 * ~1m40s using a queue. */
bool
PkgDb::scrape( nix::SymbolTable & syms,
               const Target &     target,
               std::size_t        pageSize,
               std::size_t        pageIdx )
{
  const auto & [prefix, cursor, parentId] = target;

  /* If it has previously been scraped then bail out. */
  if ( this->completedAttrSet( parentId ) ) { return true; }

  /* Store the subtree we are in for later use in various logic */
  auto subtree = Subtree( prefix.front() );

  debugLog( nix::fmt( "evaluating package set '%s'",
                      concatStringsSep( ".", prefix ) ) );

  auto   allAttribs   = cursor->getAttrs();
  size_t startIdx     = pageIdx * pageSize;
  size_t thisPageSize = startIdx + pageSize < allAttribs.size()
                          ? pageSize
                          : allAttribs.size() % pageSize;
  bool   lastPage     = thisPageSize < pageSize;
  auto   page
    = std::views::counted( allAttribs.begin() + startIdx, thisPageSize );
  Todos todo;

  for ( nix::Symbol & aname : page )
    {
      if ( syms[aname] == "recurseForDerivations" ) { continue; }

      /* Try processing this attribute.
       * If we are to recurse, todo will be loaded with the first target for
       * us... we process this subtree completely using the todo stack. */
      processSingleAttrib( syms[aname],
                           cursor->getAttr( aname ),
                           prefix,
                           parentId,
                           subtree,
                           todo );
      if ( ! todo.empty() )
        {
          const auto [parentPrefix, _a, _b] = todo.top();
          while ( ! todo.empty() )
            {
              const auto [prefix, cursor, parentId] = todo.top();
              todo.pop();

              try
                {
                  for ( nix::Symbol & aname : cursor->getAttrs() )
                    {
                      auto sym = syms[aname];
                      if ( sym == "recurseForDerivations" ) { continue; }
                      processSingleAttrib( sym,
                                           cursor->getAttr( aname ),
                                           prefix,
                                           parentId,
                                           subtree,
                                           todo );
                    }
                }
              catch ( const nix::EvalError & err )
                {
                  /* The `getAttrs()` call will throw this on a non-attribute
                   * set path.  They appear to be infrequent and the penalty
                   * checking each one appears to be high. Better ask for
                   * forgiveness than permission?  */
                  if ( err.info().msg.str().find( "is not an attribute set" )
                       != std::string::npos )
                    {
                      continue;
                    }
                  throw;
                }
            }

          this->setPrefixDone( parentPrefix, true );
        }
    }

  if ( lastPage ) { this->setPrefixDone( prefix, true ); }
  return lastPage;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
