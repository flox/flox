/* ========================================================================== *
 *
 * @file flox/search/params.hh
 *
 * @brief A set of user inputs used to set input preferences and query
 *        parameters during search.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <nlohmann/json_fwd.hpp>
#include <optional>
#include <string>
#include <string_view>
#include <variant>

#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"
#include "flox/core/util.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/registry.hh"
#include "flox/resolver/environment.hh"
#include "flox/resolver/lockfile.hh"
#include "flox/resolver/manifest.hh"


/* -------------------------------------------------------------------------- */

namespace flox::search {

/* -------------------------------------------------------------------------- */

/**
 * @brief A set of query parameters.
 *
 * This is essentially a reorganized form of @a flox::pkgdb::PkgQueryArgs
 * that is suited for JSON input.
 */
struct SearchQuery
{

  std::optional<std::string> name;    /**< Filter results by exact `name`. */
  std::optional<std::string> pname;   /**< Filter results by exact `pname`. */
  std::optional<std::string> version; /**< Filter results by exact version. */
  std::optional<std::string> semver;  /**< Filter results by version range. */

  /** Filter results by partial match on pname, attrName, or description */
  std::optional<std::string> partialMatch;

  /** Filter results by partial match on pname or attrName */
  std::optional<std::string> partialNameMatch;

  /** @brief Reset to default state. */
  void
  clear();

  /** @brief Check validity of fields, throwing an exception if invalid. */
  void
  check() const;

  /**
   * @brief Fill a @a flox::pkgdb::PkgQueryArgs struct with preferences to
   *        lookup packages filtered by @a SearchQuery requirements.
   *
   * NOTE: This DOES NOT clear @a pqa before filling it.
   * This is intended to be used after filling @a pqa with global preferences.
   * @param pqa A set of query args to _fill_ with preferences.
   * @return A reference to the modified query args.
   */
  pkgdb::PkgQueryArgs &
  fillPkgQueryArgs( pkgdb::PkgQueryArgs & pqa ) const;


}; /* End struct "SearchQuery' */


/* -------------------------------------------------------------------------- */

/**
 * @class flox::search::ParseSearchQueryException
 * @brief An exception thrown when parsing @a flox::search::SearchQuery
 *        from JSON.
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( ParseSearchQueryException,
                       EC_PARSE_SEARCH_QUERY,
                       "error parsing search query" )
/** @} */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::search::SearchQuery. */
void
from_json( const nlohmann::json & jfrom, SearchQuery & qry );

/** @brief Convert a @a flox::search::SearchQuery to a JSON object. */
void
to_json( nlohmann::json & jto, const SearchQuery & qry );


/* -------------------------------------------------------------------------- */

struct SearchParams
{

  /**
   * @brief The absolute @a std::filesystem::path to a manifest file or an
   *        inline @a flox::resolver::GlobalManifestRaw.
   */
  std::optional<
    std::variant<std::filesystem::path, resolver::GlobalManifestRaw>>
    globalManifest;

  /**
   * @brief The absolute @a std::filesystem::path to a manifest file or an
   *        inline @a flox::resolver::ManifestRaw.
   */
  std::optional<std::variant<std::filesystem::path, resolver::ManifestRaw>>
    manifest;

  /**
   * @brief The absolute @a std::filesystem::path to a lockfile or an inline
   *        @a flox::resolver::LockfileRaw.
   */
  std::optional<std::variant<std::filesystem::path, resolver::LockfileRaw>>
    lockfile;

  /**
   * @brief The @a flox::search::SearchQuery specifying the package to
   *        search for.
   */
  SearchQuery query;


  /**
   * @brief If `global-manifest` is inlined or unset, returns `std::nullopt`.
   *        Otherwise returns the path to the global manifest.
   */
  [[nodiscard]] std::optional<std::filesystem::path>
  getGlobalManifestPath();

  /**
   * @brief Returns a @a flox::resolver::GlobalManifestRaw or lazily
   *        loads it from disk ( if provided ).
   */
  [[nodiscard]] std::optional<flox::resolver::GlobalManifestRaw>
  getGlobalManifestRaw();

  /**
   * @brief If `manifest` is inlined or unset, returns `std::nullopt`.
   *        Otherwise returns the path to the manifest.
   */
  [[nodiscard]] std::optional<std::filesystem::path>
  getManifestPath();

  /**
   * @brief Returns a @a flox::resolver::ManifestRaw or lazily
   *        loads it from disk.
   *        If @a manifestPath is unset, this returns an empty manifest.
   */
  [[nodiscard]] flox::resolver::ManifestRaw
  getManifestRaw();

  /**
   * @brief If `lockfile` is inlined or unset, returns `std::nullopt`.
   *        Otherwise returns the path to the lockfile.
   */
  [[nodiscard]] std::optional<std::filesystem::path>
  getLockfilePath();

  /**
   * @brief Returns a @a flox::resolver::LockfileRaw or lazily
   *        loads it from disk ( if provided ).
   */
  [[nodiscard]] std::optional<flox::resolver::LockfileRaw>
  getLockfileRaw();


}; /* End struct `SearchParams' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::search::SearchParams. */
void
from_json( const nlohmann::json & jfrom, SearchParams & params );

/** @brief Convert a @a flox::search::SearchParams to a JSON object. */
void
to_json( nlohmann::json & jto, const SearchParams & params );


/* -------------------------------------------------------------------------- */

}  // namespace flox::search


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
