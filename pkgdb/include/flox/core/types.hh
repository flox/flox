/* ========================================================================== *
 *
 * @file flox/core/types.hh
 *
 * @brief Miscellaneous typedefs and aliases
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <string>
#include <vector>

#include <nix/eval-cache.hh>
#include <nix/flake/flake.hh>
#include <nix/ref.hh>

#include <nlohmann/json.hpp>


/* -------------------------------------------------------------------------- */

/** @brief Interfaces for use by `flox`. */
namespace flox {

/* -------------------------------------------------------------------------- */

/**
 * @brief A list of key names addressing a location in a nested
 *        JSON-like object.
 */
using AttrPath = std::vector<std::string>;

/**
 * @brief An attribute path which may contain `null` members to
 *        represent _globs_.
 *
 * Globs may only appear as the second element representing `system`.
 */
using AttrPathGlob = std::vector<std::optional<std::string>>;

/**
 * @brief A `std::shared_ptr<nix::eval_cache::AttrCursor>` which may
 *        be `nullptr`.
 */
using MaybeCursor = std::shared_ptr<nix::eval_cache::AttrCursor>;

/** @brief A non-`nullptr` `std::shared_ptr<nix::eval_cache::AttrCursor>`. */
using Cursor = nix::ref<nix::eval_cache::AttrCursor>;


/* -------------------------------------------------------------------------- */

// TODO: Make this a real type/enum.
/**
 * @brief A system pair indicating architecture and platform.
 *
 * Examples:
 *   `x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, or `aarch64-darwin`
 */
using System = std::string;


/* -------------------------------------------------------------------------- */

/** @brief A _top level_ key in a `nix` flake */
enum subtree_type { ST_NONE = 0, ST_LEGACY = 1, ST_PACKAGES = 2 };

/**
 * @fn void from_json( const nlohmann::json & j, subtree_type & pdb )
 * @brief Convert a JSON string to a @a flox::subtree_type.
 *
 * @fn void to_json( nlohmann::json & j, const subtree_type & pdb )
 * @brief Convert a @a flox::subtree_type to a JSON string.
 */
/* Generate `to_json' and `from_json' for enum. */
NLOHMANN_JSON_SERIALIZE_ENUM( subtree_type,
                              { { ST_NONE, nullptr },
                                { ST_LEGACY, "legacyPackages" },
                                { ST_PACKAGES, "packages" } } )


/** @brief A strongly typed wrapper over an attribute path _subtree_ name, which
 * is the first element of an attribute path. */
struct Subtree
{

  subtree_type subtree = ST_NONE;

  constexpr Subtree() = default;

  // NOLINTNEXTLINE
  constexpr Subtree( subtree_type subtree ) : subtree( subtree ) {}

  /** @brief Construct a @a flox::Subtree from a string. */
  constexpr explicit Subtree( std::string_view str ) noexcept
    : subtree( ( str == "legacyPackages" ) ? ST_LEGACY
               : ( str == "packages" )     ? ST_PACKAGES
                                           : ST_NONE )
  {}


  /** @brief Parse a string into a @a flox::Subtree. */
  [[nodiscard]] static Subtree
  parseSubtree( std::string_view str )
  {
    return Subtree { ( str == "legacyPackages" ) ? ST_LEGACY
                     : ( str == "packages" )
                       ? ST_PACKAGES
                       : throw std::invalid_argument(
                         "Invalid subtree '" + std::string( str ) + "'" ) };
  }


  /** @brief Convert a @a flox::Subtree to a string. */
  [[nodiscard]] friend constexpr std::string_view
  to_string( const Subtree & subtree )
  {
    switch ( subtree.subtree )
      {
        case ST_LEGACY: return "legacyPackages";
        case ST_PACKAGES: return "packages";
        default: return "ST_NONE";
      }
  }

  /** @brief Implicitly convert a @a flox::Subtree to a string. */
  constexpr explicit operator std::string_view() const
  {
    return to_string( *this );
  }

  // NOLINTNEXTLINE
  constexpr operator subtree_type() const { return this->subtree; }

  /** @brief Compare two @a flox::Subtree for equality. */
  [[nodiscard]] constexpr bool
  operator==( const Subtree & other ) const
    = default;

  /** @brief Compare two @a flox::Subtree for inequality. */
  [[nodiscard]] constexpr bool
  operator!=( const Subtree & other ) const
    = default;

  /** @brief Compare with a @a flox::subtree_type for equality. */
  [[nodiscard]] constexpr bool
  operator==( const subtree_type & other ) const
  {
    return this->subtree == other;
  }

  /** @brief Compare with a @a flox::subtree_type for inequality. */
  [[nodiscard]] constexpr bool
  operator!=( const subtree_type & other ) const
  {
    return this->subtree != other;
  }

}; /* End struct `Subtree' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON string to a @a flox::Subtree. */
inline void
from_json( const nlohmann::json & jfrom, Subtree & subtree )
{
  jfrom.get_to( subtree.subtree );
}

/** @brief Convert a @a flox::Subtree to a JSON string. */
inline void
to_json( nlohmann::json & jto, const Subtree & subtree )
{
  to_json( jto, subtree.subtree );
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
