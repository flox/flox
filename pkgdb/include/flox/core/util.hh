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

/**
 * @brief Extension to the `nlohmann::json' serializer to support additional
 *        _Argument Dependent Lookup_ (ADL) types.
 */
namespace nlohmann {

/* -------------------------------------------------------------------------- */

/** @brief Variants ( Eithers ) of two elements to/from JSON. */
template<typename A, typename B>
struct adl_serializer<std::variant<A, B>>
{

  /** @brief Convert a @a std::variant<A, B> to a JSON type. */
  static void
  to_json( json & jto, const std::variant<A, B> & var )
  {
    if ( std::holds_alternative<A>( var ) ) { jto = std::get<A>( var ); }
    else { jto = std::get<B>( var ); }
  }

  /** @brief Convert a JSON type to a @a std::variant<A, B>. */
  static void
  from_json( const json & jfrom, std::variant<A, B> & var )
  {
    try
      {
        var = jfrom.template get<A>();
      }
    catch ( ... )
      {
        var = jfrom.template get<B>();
      }
  }

}; /* End struct `adl_serializer<std::variant<A, B>>' */


/* -------------------------------------------------------------------------- */

/**
 * @brief Variants ( Eithers ) of any number of elements to/from JSON.
 *
 * The order of your types effects priority.
 * Any valid parse or coercion from a type named _early_ in the variant list
 * will succeed before attempting to parse alternatives.
 *
 * For example, always attempt `bool` first, then `int`, then `float`, and
 * alway attempt `std::string` LAST.
 *
 * It's important to note that you must never nest multiple `std::optional`
 * types in a variant, instead make `std::optional<std::variant<...>>`.
 */
template<typename A, typename... Types>
struct adl_serializer<std::variant<A, Types...>>
{

  /** @brief Convert a @a std::variant<A, Types...> to a JSON type. */
  static void
  to_json( json & jto, const std::variant<A, Types...> & var )
  {
    /* This _unwraps_ the inner type and calls the proper `to_json'.
     * The compiler does the heavy lifting for us here <3. */
    std::visit( [&]( auto unwrapped ) -> void { jto = unwrapped; }, var );
  }

  /** @brief Convert a JSON type to a @a std::variant<Types...>. */
  static void
  from_json( const json & jfrom, std::variant<A, Types...> & var )
  {
    /* Try getting typename `A', or recur. */
    try
      {
        var = jfrom.template get<A>();
      }
    catch ( ... )
      {
        /* Strip typename `A' from variant, and call recursively. */
        using next_variant = std::variant<Types...>;

        /* Coerce to `next_variant' type. */
        next_variant next = jfrom.template get<next_variant>();
        std::visit( [&]( auto unwrapped ) -> void { var = unwrapped; }, next );
      }
  }

}; /* End struct `adl_serializer<std::variant<A, Types...>>' */


/* -------------------------------------------------------------------------- */

/** @brief @a nix::fetchers::Attrs to/from JSON */
template<>
struct adl_serializer<nix::fetchers::Attrs>
{

  /** @brief Convert a @a nix::fetchers::Attrs to a JSON object. */
  static void
  to_json( json & jto, const nix::fetchers::Attrs & attrs )
  {
    /* That was easy. */
    jto = nix::fetchers::attrsToJSON( attrs );
  }

  /** @brief Convert a JSON object to a @a nix::fetchers::Attrs. */
  static void
  from_json( const json & jfrom, nix::fetchers::Attrs & attrs )
  {
    /* That was easy. */
    attrs = nix::fetchers::jsonToAttrs( jfrom );
  }

}; /* End struct `adl_serializer<nix::fetchers::Attrs>' */


/* -------------------------------------------------------------------------- */

/** @brief @a nix::FlakeRef to/from JSON. */
template<>
struct adl_serializer<nix::FlakeRef>
{

  /** @brief Convert a @a nix::FlakeRef to a JSON object. */
  static void
  to_json( json & jto, const nix::FlakeRef & ref )
  {
    /* That was easy. */
    jto = nix::fetchers::attrsToJSON( ref.toAttrs() );
  }

  /** @brief _Move-only_ conversion of a JSON object to a @a nix::FlakeRef. */
  [[nodiscard]] static nix::FlakeRef
  from_json( const json & jfrom )
  {
    if ( jfrom.is_object() )
      {
        auto attrs = nix::fetchers::jsonToAttrs( jfrom );
        return nix::FlakeRef::fromAttrs( attrs );
        ;
      }
    return { nix::parseFlakeRef( jfrom.get<std::string>() ) };
  }

}; /* End struct `adl_serializer<nix::FlakeRef>' */


/* -------------------------------------------------------------------------- */

}  // namespace nlohmann


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

/** @brief Convert a YAML string to JSON. */
[[nodiscard]] nlohmann::json
yamlToJSON( std::string_view yaml );


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
 * @brief Does the string @str have the prefix @a prefix?
 * @param prefix The prefix to check for.
 * @param str String to test.
 * @return `true` iff @a str has the prefix @a prefix.
 */
[[nodiscard]] bool
hasPrefix( std::string_view prefix, std::string_view str );


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
      if ( rsl.size() != 0 ) { rsl += sep; }
      rsl += idx;
    }
  return rsl;
}


/* -------------------------------------------------------------------------- */

/** @brief Print a log message with the provided log level. */
void
printLog( const nix::Verbosity & lvl, const std::string & msg );

/** @brief Prints a log message to `stderr` when called with `-vvvv`. */
void
traceLog( const std::string & msg );

/**
 * @brief Prints a log message to `stderr` when called with `--debug` or `-vvv`.
 */
void
debugLog( const std::string & msg );

/** @brief Prints a log message to `stderr` at default verbosity. */
void
infoLog( const std::string & msg );

/** @brief Prints a log message to `stderr` when verbosity is at least `-q`. */
void
warningLog( const std::string & msg );

/** @brief Prints a log message to `stderr` when verbosity is at least `-qq`. */
void
errorLog( const std::string & msg );


/* -------------------------------------------------------------------------- */

/** @brief Returns true if the flake reference points to a nixpkgs revision. */
bool
isNixpkgsRef( nix::FlakeRef const & ref );


/* -------------------------------------------------------------------------- */

static std::string const FLOX_FLAKE_TYPE = "flox-nixpkgs";

/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
