/* ========================================================================== *
 *
 * @file flox/core/util.hh
 *
 * @brief Miscellaneous helper functions.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <initializer_list>
#include <sstream>
#include <string>  // For `std::string' and `std::string_view'
#include <string_view>
#include <variant>
#include <vector>

#include <nix/attrs.hh>
#include <nix/error.hh>
#include <nix/flake/flakeref.hh>
#include <nix/users.hh>
#include <nix/util.hh>
#include <nlohmann/json.hpp>

#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"


/* -------------------------------------------------------------------------- */

/* Backported from C++20a for C++20b compatability. */

/**
 * @brief Helper type for `std::visit( overloaded { ... }, x );` pattern.
 *
 * This is a _quality of life_ helper that shortens boilerplate required for
 * creating type matching statements.
 */
template<class... Ts>
struct overloaded : Ts...
{
  using Ts::operator()...;
};

template<class... Ts>
overloaded( Ts... ) -> overloaded<Ts...>;


/* -------------------------------------------------------------------------- */

/** @brief Detect if two vectors are equal. */
template<typename T>
[[nodiscard]] bool
operator==( const std::vector<T> & lhs, const std::vector<T> & rhs )
{
  if ( lhs.size() != rhs.size() ) { return false; }
  for ( size_t idx = 0; idx < lhs.size(); ++idx )
    {
      if ( lhs[idx] != rhs[idx] ) { return false; }
    }
  return true;
}

/** @brief Detect if two vectors are not equal. */
template<typename T>
[[nodiscard]] bool
operator!=( const std::vector<T> & lhs, const std::vector<T> & rhs )
{
  return ! ( lhs == rhs );
}


/* -------------------------------------------------------------------------- */

/** @brief Detect if two vectors of strings are equal. */
[[nodiscard]] bool
operator==( const std::vector<std::string> & lhs,
            const std::vector<std::string> & rhs );


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/** @brief Systems to resolve/search in. */
[[nodiscard]] inline static const std::vector<std::string> &
getDefaultSystems()
{
  static const std::vector<std::string> defaultSystems
    = { "x86_64-linux", "aarch64-linux", "x86_64-darwin", "aarch64-darwin" };
  return defaultSystems;
}


/** @brief `flake' subtrees to resolve/search in. */
[[nodiscard]] inline static const std::vector<std::string> &
getDefaultSubtrees()
{
  static const std::vector<std::string> defaultSubtrees
    = { "packages", "legacyPackages" };
  return defaultSubtrees;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Detect if a path is a SQLite3 database file.
 * @param dbPath Absolute path.
 * @return `true` iff @a path is a SQLite3 database file.
 */
[[nodiscard]] bool
isSQLiteDb( const std::string & dbPath );


/* -------------------------------------------------------------------------- */

/**
 * @brief Predicate to detect failing SQLite3 return codes.
 * @param rcode A SQLite3 _return code_.
 * @return `true` iff @a rcode is a SQLite3 error.
 */
bool
isSQLError( int rcode );


/* -------------------------------------------------------------------------- */

/**
 * @brief Parse a flake reference from either a JSON attrset or URI string.
 * @param flakeRef JSON or URI string representing a `nix` flake reference.
 * @return Parsed flake reference object.
 */
[[nodiscard]] nix::FlakeRef
parseFlakeRef( const std::string & flakeRef );


/* -------------------------------------------------------------------------- */

/**
 * @brief Parse a JSON object from an inline string or a path to a JSON file.
 * @param jsonOrPath A JSON string or a path to a JSON file.
 * @return A parsed JSON object.
 */
[[nodiscard]] nlohmann::json
parseOrReadJSONObject( const std::string & jsonOrPath );


/* -------------------------------------------------------------------------- */

/** @brief Convert a TOML string to JSON. */
[[nodiscard]] nlohmann::json
tomlToJSON( std::string_view toml );


/* -------------------------------------------------------------------------- */

/**
 * @brief Read a file and coerce its contents to JSON based on its extension.
 *
 * Files with the extension `.json` are parsed directly.
 * Files with the extension `.yaml` or `.yml` are converted to JSON from YAML.
 * Files with the extension `.toml` are converted to JSON from TOML.
 */
[[nodiscard]] nlohmann::json
readAndCoerceJSON( const std::filesystem::path & path );


/* -------------------------------------------------------------------------- */

/**
 * @brief Split an attribute path string.
 *
 * Handles quoted strings and escapes.
 */
[[nodiscard]] std::vector<std::string>
splitAttrPath( std::string_view path );


/* -------------------------------------------------------------------------- */

/**
 * @brief Is the string @str a positive natural number?
 * @param str String to test.
 * @return `true` iff @a str is a stringized unsigned integer.
 */
[[nodiscard]] bool
isUInt( std::string_view str );


/* -------------------------------------------------------------------------- */

/**
 * @brief Does the string @a str have the prefix @a prefix?
 * @param prefix The prefix to check for.
 * @param str String to test.
 * @return `true` iff @a str has the prefix @a prefix.
 */
[[nodiscard]] bool
hasPrefix( std::string_view prefix, std::string_view str );

/* -------------------------------------------------------------------------- */

/**
 * @brief Does the vector of strings @a lst begin with the elements
 *        of @a prefix?
 * @param prefix The prefix to check for.
 * @param lst Vector of strings to test.
 * @return `true` iff @a lst has the prefix @a prefix.
 */
[[nodiscard]] bool
hasPrefix( const std::vector<std::string> & prefix,
           const std::vector<std::string> & lst );


/* -------------------------------------------------------------------------- */

/** @brief trim from start ( in place ). */
std::string &
ltrim( std::string & str );

/** @brief trim from end ( in place ). */
std::string &
rtrim( std::string & str );

/** @brief trim from both ends ( in place ). */
std::string &
trim( std::string & str );


/** @brief trim from start ( copying ). */
[[nodiscard]] std::string
ltrim_copy( std::string_view str );

/** @brief trim from end ( copying ). */
[[nodiscard]] std::string
rtrim_copy( std::string_view str );

/** @brief trim from both ends ( copying ). */
[[nodiscard]] std::string
trim_copy( std::string_view str );


/* -------------------------------------------------------------------------- */

/**
 * @brief Extract the user-friendly portion of a @a nlohmann::json::exception.
 */
[[nodiscard]] std::string
extract_json_errmsg( nlohmann::json::exception & err );

/* -------------------------------------------------------------------------- */

/**
 * @brief Assert that a JSON value is an object, or throw an exception.
 *
 * The type of exception and an optional _path_ for messages can be provided.
 */
template<typename Exception = FloxException>
static void
assertIsJSONObject( const nlohmann::json & value,
                    const std::string &    who = "JSON value" )
{
  if ( ! value.is_object() )
    {
      std::stringstream oss;
      oss << "expected " << who << " to be an object, but found "
          << ( value.is_array() ? "an" : "a" ) << ' ' << value.type_name()
          << '.';
      throw Exception( oss.str() );
    }
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Merge two @a std::vector containers by putting all elements of the
 *        higher prioirty vector first, then appending the deduplicated keys of
 *        the lower priortity vector.
 * @param lower The lower priority @a std::vector.
 * @param higher The higher priority @a std::vector.
 * @return The merged @a std::vector.
 */
template<typename T>
[[nodiscard]] std::vector<T>
merge_vectors( const std::vector<T> & lower, const std::vector<T> & higher )
{
  std::vector<T> merged = higher;
  for ( const auto & value : lower )
    {
      if ( std::find( merged.begin(), merged.end(), value ) == merged.end() )
        {
          merged.emplace_back( value );
        }
    }
  return merged;
}


/* -------------------------------------------------------------------------- */

/** @brief Convert a @a AttrPathGlob to a string for display. */
[[nodiscard]] std::string
displayableGlobbedPath( const AttrPathGlob & attrs );

/** @brief Get available system memory in kb */
[[nodiscard]] long
getAvailableSystemMemory();

/** @brief Get the main flox cache dir */
std::filesystem::path
getFloxCachedir();


/* -------------------------------------------------------------------------- */

/**
 * @brief Concatenate the given strings with a separator between
 *        the elements.
 */
template<class Container>
[[nodiscard]] std::string
concatStringsSep( const std::string_view sep, const Container & strings )
{
  size_t size = 0;
  /* Needs a cast to string_view since this is also called
   * with `nix::Symbols'. */
  for ( const auto & str : strings )
    {
      size += sep.size() + std::string_view( str ).size();
    }
  std::string rsl;
  rsl.reserve( size );
  for ( auto & idx : strings )
    {
      if ( ! rsl.empty() ) { rsl += sep; }
      rsl += idx;
    }
  return rsl;
}


/* -------------------------------------------------------------------------- */

/** @brief Print a log message with the provided log level.
 *
 * This is a macro so that any allocations needed for msg can be optimized out.
 */
#define printLog( lvl, msg )                                                                               \
  /* See                                                                                                   \
   * https://github.com/NixOS/nix/blob/09a6e8e7030170611a833612b9f40b9a10778c18/src/libutil/logging.cc#L64 \
   * for lvl to verbosity comparison                                                                       \
   */                                                                                                      \
  if ( ! ( ( lvl ) > nix::verbosity ) ) { nix::logger->log( lvl, msg ); }

/** @brief Prints a log message to `stderr` when called with `-vvvv`. */
#define traceLog( msg ) printLog( nix::Verbosity::lvlVomit, msg )

/**
 * @brief Prints a log message to `stderr` when called with `--debug` or `-vvv`.
 */
#define debugLog( msg ) printLog( nix::Verbosity::lvlDebug, msg )

/**
 * @brief Prints a log message to `stderr` when called with `--verbose` or `-v`.
 */
#define verboseLog( msg ) printLog( nix::Verbosity::lvlTalkative, msg )

/** @brief Prints a log message to `stderr` at default verbosity. */
#define infoLog( msg ) printLog( nix::Verbosity::lvlInfo, msg )

/** @brief Prints a log message to `stderr` when verbosity is at least `-q`. */
#define warningLog( msg ) printLog( nix::Verbosity::lvlWarn, msg )

/** @brief Prints a log message to `stderr` when verbosity is at least `-qq`. */
#define errorLog( msg ) printLog( nix::Verbosity::lvlError, msg )

/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
