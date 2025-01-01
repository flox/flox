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
#include <nix/util.hh>
#include <nlohmann/json.hpp>

/* -------------------------------------------------------------------------- */

/* Backported from C++20a for C++20b compatibility. */

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
 * always attempt `std::string` LAST.
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
        return { nix::FlakeRef::fromAttrs(
          nix::fetchers::jsonToAttrs( jfrom ) ) };
      }
    return { nix::parseFlakeRef( jfrom.get<std::string>() ) };
  }

}; /* End struct `adl_serializer<nix::FlakeRef>' */


/* -------------------------------------------------------------------------- */

}  // namespace nlohmann

/* -------------------------------------------------------------------------- */

/** @brief Detect if two vectors of strings are equal. */
[[nodiscard]] bool
operator==( const std::vector<std::string> & lhs,
            const std::vector<std::string> & rhs );


/* -------------------------------------------------------------------------- */

namespace flox {

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
