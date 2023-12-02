/* ========================================================================== *
 *
 * @file flox/pkgdb/pkg-query.hh
 *
 * @brief Interfaces for constructing complex `Packages' queries.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <cstdint>
#include <memory>
#include <optional>
#include <sstream>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>

#include <nix/config.hh>
#include <nix/globals.hh>
#include <nlohmann/json.hpp>

#include "compat/concepts.hh"
#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"


/* -------------------------------------------------------------------------- */

/* Forward Declarations. */

namespace sqlite3pp {
class database;
class query;
}  // namespace sqlite3pp


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

using row_id = uint64_t; /**< A _row_ index in a SQLite3 table. */


/* -------------------------------------------------------------------------- */

/**
 * @class flox::pkgdb::InvalidPkgQueryArg
 * @brief Indicates invalid arguments were set in a
 *        @a flox::resolver::PkgQueryArgs struct.
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( InvalidPkgQueryArg,
                       EC_INVALID_PKG_QUERY_ARG,
                       "invalid package query argument" )
/** @} */


/* -------------------------------------------------------------------------- */

/**
 * @brief Collection of query parameters used to lookup packages in a database.
 *
 * These use a combination of SQL statements and post processing with
 * `node-semver` to produce a list of satisfactory packages.
 */
struct PkgQueryArgs
{

  std::optional<std::string> name;    /**< Filter results by exact `name`. */
  std::optional<std::string> pname;   /**< Filter results by exact `pname`. */
  std::optional<std::string> version; /**< Filter results by exact version. */
  std::optional<std::string> semver;  /**< Filter results by version range. */

  /** Filter results by partial match on pname, attrName, or description. */
  std::optional<std::string> partialMatch;

  /** Filter results by partial match on pname or attrName. */
  std::optional<std::string> partialNameMatch;

  /** Filter results by an exact match on either `pname` or `attrName`. */
  std::optional<std::string> pnameOrAttrName;

  /**
   * Filter results to those explicitly marked with the given licenses.
   *
   * NOTE: License strings should be SPDX Ids ( short names ).
   */
  std::optional<std::vector<std::string>> licenses;

  /** Whether to include packages which are explicitly marked `broken`. */
  bool allowBroken = false;

  /** Whether to include packages which are explicitly marked `unfree`. */
  bool allowUnfree = true;

  /** Whether pre-release versions should be ordered before releases. */
  bool preferPreReleases = false;

  /**
   * Subtrees to search.
   *
   * NOTE: `Subtree` is an enum of top level flake outputs, being one of
   * `"packages"` or `"legacyPackages"`.
   */
  std::optional<std::vector<Subtree>> subtrees;

  /** Systems to search. Defaults to the current system. */
  std::vector<System> systems = { nix::settings.thisSystem.get() };

  /**
   * Relative attribute path to package from its prefix.
   * it is the part following `system`.
   *
   * NOTE: @a flox::AttrPath is an alias of `std::vector<std::string>`.
   */
  std::optional<flox::AttrPath> relPath;


  /** @brief Reset argset to its _default_ state. */
  void
  clear();

  /**
   * @brief Sanity check parameters throwing a
   *        @a flox::pkgdb::InvalidPkgQueryArgs exception if they are invalid.
   *
   * Make sure `systems` are valid systems.
   * Make sure `name` is not set when `pname`, `version`, or `semver` are set.
   * Make sure `version` is not set when `semver` is set.
   * @return `std::nullopt` iff the above conditions are met, an error
   *         code otherwise.
   */
  void
  check() const;


}; /* End struct `PkgQueryArgs' */

/* -------------------------------------------------------------------------- */

/**
 * @brief Convert an @a flox::pkgdb::PkgQueryArgs to a
 *              JSON object.
 */
void
to_json( nlohmann::json & jto, const PkgQueryArgs & descriptor );


/* -------------------------------------------------------------------------- */

/**
 * @brief A query used to lookup packages in a database.
 *
 * This uses a combination of SQL statements and post processing with
 * `node-semver` to produce a list of satisfactory packages.
 */
class PkgQuery : public PkgQueryArgs
{

private:

  /** Stream used to build up the `SELECT` block. */
  std::stringstream selects;
  /** Indicates if @a selects is empty so we know whether to add separator. */
  bool firstSelect = true;

  /** Stream used to build up the `ORDER BY` block. */
  std::stringstream orders;
  /** Indicates if @a orders is empty so we know whether to add separator. */
  bool firstOrder = true;

  /** Stream used to build up the `WHERE` block. */
  std::stringstream wheres;
  /** Indicates if @a wheres is empty so we know whether to add separator. */
  bool firstWhere = true;

  /** `( <PARAM-NAME>, <VALUE> )` pairs that need to be _bound_ by SQLite3. */
  std::unordered_map<std::string, std::string> binds;

  /**
   * Final set of columns to expose after all filtering and ordering has been
   * performed on temporary fields.
   * The value `*` may be used to export all fields.
   *
   * This setting is only intended for use by unit tests, any columns other
   * than `id` and `semver` may be changed without being reflected in normal
   * `pkgdb` semantic version updates.
   */
  std::vector<std::string> exportedColumns = { "id", "semver" };


  /* Member Functions */

  /**
   * @brief Clear member @a PkgQuery member variables of any state from past
   *        initialization runs.
   *
   * This is called by @a init before translating
   * @a flox::pkgdb::PkgQueryArgs members.
   */
  void
  clearBuilt();

  /**
   * @brief Add a new column to the _inner_ `SELECT` statement.
   *
   * These selections may be used internally for filtering and ordering rows,
   * and are only _exported_ in the final result if they are also listed
   * in @a exportedColumns.
   * @param column A column `SELECT` statement such as `v_PackagesSearch.id`
   *               or `0 AS foo`.
   */
  void
  addSelection( std::string_view column );

  /** @brief Appends the `ORDER BY` block. */
  void
  addOrderBy( std::string_view order );

  /**
   * @brief Appends the `WHERE` block with a new `AND ( <COND> )` statement.
   */
  void
  addWhere( std::string_view cond );

  /**
   * @brief Filter a set of semantic version numbers by the range indicated in
   *        the @a semvers member variable.
   *
   * If @a semvers is unset, return the original set _as is_.
   */
  [[nodiscard]] std::unordered_set<std::string>
  filterSemvers( const std::unordered_set<std::string> & versions ) const;

  /** @brief A helper of @a init() which handles `match` filtering/ranking. */
  void
  initMatch();

  /**
   * @brief A helper of @a init() which handles `subtrees` filtering/ranking.
   */
  void
  initSubtrees();

  /**
   * @brief A helper of @a init() which handles `systems` filtering/ranking.
   */
  void
  initSystems();

  /** @brief A helper of @a init() which constructs the `ORDER BY` block. */
  void
  initOrderBy();

  /**
   * @brief Translate @a floco::pkgdb::PkgQueryArgs parameters to a _built_
   *        SQL statement held in `std::stringstream` member variables.
   *
   * This is called by constructors, and should be called manually if any
   * @a flox::pkgdb::PkgQueryArgs members are manually edited.
   */
  void
  init();


public:

  PkgQuery() { this->init(); }

  explicit PkgQuery( const PkgQueryArgs & params ) : PkgQueryArgs( params )
  {
    this->init();
  }

  PkgQuery( const PkgQueryArgs &     params,
            std::vector<std::string> exportedColumns )
    : PkgQueryArgs( params ), exportedColumns( std::move( exportedColumns ) )
  {
    this->init();
  }

  /**
   * @brief Produce an unbound SQL statement from various member variables.
   *
   * This must be run after @a init().
   * The returned string still needs to be processed to _bind_ host parameters
   * from @a binds before being executed.
   * @return An unbound SQL query string.
   */
  [[nodiscard]] std::string
  str() const;

  /**
   * @brief Create a bound SQLite query ready for execution.
   *
   * This does NOT perform filtering by `semver` which must be performed as a
   * post-processing step.
   * Unlike @a execute() this routine allows the caller to iterate over rows.
   */
  [[nodiscard]] std::shared_ptr<sqlite3pp::query>
  bind( sqlite3pp::database & pdb ) const;

  /**
   * @brief Query a given database returning an ordered list of
   *        satisfactory `Packages.id`s.
   *
   * This performs `semver` filtering.
   */
  [[nodiscard]] std::vector<row_id>
  execute( sqlite3pp::database & pdb ) const;


}; /* End class `PkgQuery' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
