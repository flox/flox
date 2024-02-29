/* ========================================================================== *
 *
 * @file flox/pkgdb/write.hh
 *
 * @brief Interfaces for writing to a SQLite3 package set database.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <stack>
#include <tuple>

#include <nlohmann/json.hpp>

#include "flox/core/types.hh"
#include "flox/pkgdb/read.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/** @brief A set of arguments used by @a flox::pkgdb::PkgDb::scrape. */
using Target = std::tuple<flox::AttrPath, flox::Cursor, row_id>;

/**
 * @brief A stack of @a flox::pkgdb::Target to be completed.
 * A stack is used to promote depth-first processing.
 */
using Todos = std::stack<Target, std::list<Target>>;

/* -------------------------------------------------------------------------- */

/** @brief Scraping rules to modify database creation process in _raw_ form. */
struct ScrapeRulesRaw
{
  std::vector<AttrPathGlob> allowPackage;
  std::vector<AttrPathGlob> disallowPackage;
  std::vector<AttrPathGlob> allowRecursive;
  std::vector<AttrPathGlob> disallowRecursive;
}; /* End struct `ScrapeRulesRaw` */


/** @brief Convert a JSON object to a @a flox::pkgdb::ScrapeRulesRaw. */
void
from_json( const nlohmann::json & jfrom, ScrapeRulesRaw & rules );


/* -------------------------------------------------------------------------- */

enum ScrapeRule {
  SR_NONE = 0,         /**< Empty state. */
  SR_DEFAULT,          /**< Applies no special rules. */
  SR_ALLOW_PACKAGE,    /**< Forces an package entry in DB. */
  SR_ALLOW_RECURSIVE,  /**< Forces a sub-tree to be scraped. */
  SR_DISALLOW_PACKAGE, /**< Do not add package entry to DB. */
  /** Ignore sub-tree members unless otherwise specified. */
  SR_DISALLOW_RECURSIVE
}; /* End enum `ScrapeRule` */

[[nodiscard]] std::string
scrapeRuleToString( ScrapeRule rule );


/* -------------------------------------------------------------------------- */
struct RulesTreeNode
{
  using Children = std::unordered_map<std::string, RulesTreeNode>;

  std::string attrName = "";
  ScrapeRule  rule     = SR_DEFAULT;
  Children    children = {};

  RulesTreeNode() = default;

  explicit RulesTreeNode( ScrapeRulesRaw rules );

  explicit RulesTreeNode( const std::filesystem::path & path )
    : RulesTreeNode( static_cast<ScrapeRulesRaw>( readAndCoerceJSON( path ) ) )
  {}

  explicit RulesTreeNode( std::string attrName,
                          ScrapeRule  rule     = SR_DEFAULT,
                          Children    children = {} )
    : attrName( std::move( attrName ) )
    , rule( std::move( rule ) )
    , children( std::move( children ) )
  {}

  RulesTreeNode( std::string attrName, Children children )
    : attrName( std::move( attrName ) ), children( std::move( children ) )
  {}

  void
  addRule( AttrPathGlob & relPath, ScrapeRule rule );

  /**
   * @brief Get the rule at a path, or @a flox::pkgdb::SR_DEFAULT as a fallback.
   *
   * This *does NOT* apply parent rules to children.
   *
   * @see @a flox::pkgdb::RulesTreeNode::applyRules
   */
  [[nodiscard]] ScrapeRule
  getRule( const AttrPath & path = {} ) const;

  /**
   * @brief Return true/false for explicit allow/disallow, or `std::nullopt`
   *        if no rule is defined.
   *        This is intended for use on _root_ nodes.
   *
   * Parent paths may _pass down_ rules to children unless otherwise defined
   * at lower levels.
   */
  [[nodiscard]] std::optional<bool>
  applyRules( const AttrPath & path ) const;


}; /* End struct `RulesTreeNode' */


/** @brief Convert a JSON object to a @a flox::pkgdb::RulesTreeNode. */
void
from_json( const nlohmann::json & jfrom, RulesTreeNode & rules );

/** @brief Convert a @a flox::pkgdb::RulesTreeNode to a JSON object. */
void
to_json( nlohmann::json & jto, const RulesTreeNode & rules );


/* -------------------------------------------------------------------------- */

/**
 * @brief A SQLite3 database used to cache derivation/package information about
 *        a single locked flake.
 */
class PkgDb : public PkgDbReadOnly
{

  /* Internal Helpers */

protected:

  /**
   * @brief Create/update tables/views schema in database.
   * Create tables if they do not exist.
   * Create views in database if they do not exist or update them.
   * Create `DbVersions` rows if they do not exist.
   */
  void
  init();

  /* Constructors */

public:

  /**
   * @brief Opens an existing database.
   *
   * Does NOT attempt to create a database if one does not exist.
   * @param dbPath Absolute path to database file.
   */
  explicit PkgDb( std::string_view dbPath )
  {
    this->dbPath = dbPath;
    if ( ! std::filesystem::exists( this->dbPath ) )
      {
        throw PkgDbReadOnly::NoSuchDatabase(
          *dynamic_cast<PkgDbReadOnly *>( this ) );
      }
    this->db.connect( this->dbPath.c_str(),
                      SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE );
    this->init();
    this->loadLockedFlake();
  }

  /**
   * @brief Opens a DB directly by its fingerprint hash.
   *
   * Does NOT attempt to create a database if one does not exist.
   * @param fingerprint Unique hash associated with locked flake.
   * @param dbPath Absolute path to database file.
   */
  PkgDb( const Fingerprint & fingerprint, std::string_view dbPath )
  {
    this->dbPath      = dbPath;
    this->fingerprint = fingerprint;
    if ( ! std::filesystem::exists( this->dbPath ) )
      {
        throw PkgDbReadOnly::NoSuchDatabase(
          *dynamic_cast<PkgDbReadOnly *>( this ) );
      }
    this->db.connect( this->dbPath.c_str(),
                      SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE );
    this->init();
    this->loadLockedFlake();
  }

  /**
   * @brief Opens a DB directly by its fingerprint hash.
   *
   * Does NOT attempt to create a database if one does not exist.
   * @param fingerprint Unique hash associated with locked flake.
   */
  explicit PkgDb( const Fingerprint & fingerprint )
    : PkgDb( fingerprint, genPkgDbName( fingerprint ).string() )
  {}

  /**
   * @brief Opens a DB associated with a locked flake.
   *
   * Creates database if one does not exist.
   * @param flake Flake associated with the db. Used to write input metadata.
   * @param dbPath Absolute path to database file.
   */
  PkgDb( const nix::flake::LockedFlake & flake, std::string_view dbPath );

  /**
   * @brief Opens a DB associated with a locked flake.
   *
   * Creates database if one does not exist.
   * @param flake Flake associated with the db. Used to write input metadata.
   */
  explicit PkgDb( const nix::flake::LockedFlake & flake )
    : PkgDb( flake, genPkgDbName( flake.getFingerprint() ).string() )
  {}

  /* Connecting and locking */

  /**
   * @brief Tries to connect to the database, acquiring an exclusive lock on it.
   */
  void
  connect();


  /* Basic Operations */

  /**
   * @brief Execute a raw sqlite statement on the database.
   * @param stmt String statement to execute.
   * @return `SQLITE_*` [error code](https://www.sqlite.org/rescode.html).
   */
  inline sql_rc
  execute( const char * stmt )
  {
    sqlite3pp::command cmd( this->db, stmt );
    return cmd.execute();
  }

  /**
   * @brief Execute raw sqlite statements on the database.
   * @param stmt String statement to execute.
   * @return `SQLITE_*` [error code](https://www.sqlite.org/rescode.html).
   */
  inline sql_rc
  execute_all( const char * stmt )
  {
    sqlite3pp::command cmd( this->db, stmt );
    return cmd.execute_all();
  }

  /* Insert */

  /**
   * @brief Get the `AttrSet.id` for a given child of the attribute set
   *        associated with `parent` if it exists, or insert a new row for
   *        @a path and return its `id`.
   * @param attrName An attribute set field name.
   * @param parent The `AttrSet.id` containing @a attrName.
   *               The `id` 0 may be used to indicate that @a attrName has no
   *               parent attribute set.
   * @return A unique `row_id` ( unsigned 64bit int ) associated with
   *         @a attrName under @a parent.
   */
  row_id
  addOrGetAttrSetId( const std::string & attrName, row_id parent = 0 );

  /**
   * @brief Get the `AttrSet.id` for a given path if it exists, or insert a
   *        new row for @a path and return its `pathId`.
   * @param path An attribute path prefix such as `packages.x86_64-linux` or
   *             `legacyPackages.aarch64-darwin.python3Packages`.
   * @return A unique `row_id` ( unsigned 64bit int ) associated with @a path.
   */
  row_id
  addOrGetAttrSetId( const flox::AttrPath & path );

  /**
   * @brief Get the `Descriptions.id` for a given string if it exists, or
   *        insert a new row for @a description and return its `id`.
   * @param description A string describing a package.
   * @return A unique `row_id` ( unsigned 64bit int ) associated
   *         with @a description.
   */
  row_id
  addOrGetDescriptionId( const std::string & description );

  /**
   * @brief Adds a package to the database.
   * @param parentId The `pathId` associated with the parent path.
   * @param attrName The name of the attribute name to be added ( last element
   *                 of the attribute path ).
   * @param cursor An attribute cursor to scrape data from.
   * @return The `Packages.id` value for the added package.
   */
  row_id
  addPackage( row_id               parentId,
              std::string_view     attrName,
              const flox::Cursor & cursor );

  /* Updates */

  /**
   * @brief Update the `done` column for an attribute set and all of its
   *        children recursively.
   * @param prefixId `AttrSets.id` for the prefix to be updated.
   * @param done Value to update `done` column to.
   */
  void
  setPrefixDone( row_id prefixId, bool done );

  /**
   * @brief Update the `done` column for an attribute set and all of its
   *        children recursively.
   * @param prefix Attribute set prefix to be updated.
   * @param done Value to update `done` column to.
   */
  void
  setPrefixDone( const flox::AttrPath & prefix, bool done );

  /**
   * @brief Scrape package definitions from an attribute set.
   *
   * Processes a subset of the attribute set rooted at @a target.
   * The child attributes are chunked into pages of size @a pageSize, and
   * the @a pageIdx -th page is processed in this invocation.  Attributes are
   * processed depth first so the page is gauraunteed to be fully processed on
   * a clean return.
   *
   * @param syms Symbol table from @a cursor evaluator.
   * @param target A tuple containing the attribute path to scrape, a cursor,
   *               and a SQLite _row id_.
   * @param pageSize The size of chunks to process at a time.
   * @param pageIdx The specific page to process in this invocation.
   * @return True if the entire attribute set has been processed.
   */
  bool
  scrape( nix::SymbolTable & syms,
          const Target &     target,
          std::size_t        pageSize,
          std::size_t        pageIdx );

  /**
   * @brief Helper function for @a scrape to process a single attribute, adding
   * child attributes to the @a todo queue when appropriate to recurse.
   */
  void
  processSingleAttrib( const nix::SymbolStr &    sym,
                       const flox::Cursor &      cursor,
                       const flox::AttrPath &    prefix,
                       const flox::pkgdb::row_id parentId,
                       const flox::subtree_type  subtree,
                       Todos &                   todo );


}; /* End class `PkgDb' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
