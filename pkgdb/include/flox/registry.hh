/* ========================================================================== *
 *
 * @file flox/registry.hh
 *
 * @brief A set of user inputs used to set input preferences during search
 *        and resolution.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <algorithm>
#include <functional>
#include <map>
#include <vector>

#include <nix/fetchers.hh>
#include <nix/flake/flakeref.hh>
#include <nlohmann/json.hpp>

#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"
#include "flox/core/util.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/** @brief Preferences associated with a named registry input. */
struct RegistryInput
{

  /* From `InputPreferences':
   *   std::optional<std::vector<Subtree>> subtrees;
   */

  std::shared_ptr<nix::FlakeRef> from; /**< A parsed flake reference. */

  RegistryInput() = default;

  explicit RegistryInput( const nix::FlakeRef & from )
    : from( std::make_shared<nix::FlakeRef>( from ) )
  {}


  /** @brief Get the flake reference associated with this input. */
  [[nodiscard]] nix::ref<nix::FlakeRef>
  getFlakeRef() const
  {
    return static_cast<nix::ref<nix::FlakeRef>>( this->from );
  };

  [[nodiscard]] bool
  operator==( const RegistryInput & other ) const
  {
    if ( this->from == other.from ) { return true; }

    if ( ( this->from == nullptr ) || ( other.from == nullptr ) )
      {
        return false;
      }

    return ( *this->from ) == ( *other.from );
  }

  [[nodiscard]] bool
  operator!=( const RegistryInput & other ) const
  {
    return ! ( ( *this ) == other );
  }


}; /* End struct `RegistryInput' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::RegistryInput. */
void
from_json( const nlohmann::json & jfrom, RegistryInput & rip );

/** @brief Convert a @a flox::RegistryInput to a JSON object. */
void
to_json( nlohmann::json & jto, const RegistryInput & rip );


/* -------------------------------------------------------------------------- */

/**
 * @brief A set of user inputs used to set input preferences during search
 *        and resolution.
 *
 * Example Registry:
 * ```
 * {
 *   "inputs": {
 *     "nixpkgs": {
 *       "from": {
 *         "type": "github"
 *       , "owner": "NixOS"
 *       , "repo": "nixpkgs"
 *       }
 *     , "subtrees": ["legacyPackages"]
 *     }
 *   , "floco": {
 *       "from": {
 *         "type": "github"
 *       , "owner": "aakropotkin"
 *       , "repo": "floco"
 *       }
 *     , "subtrees": ["packages"]
 *     }
 *   }
 * , "defaults": {
 *     "subtrees": null
 *   }
 * , "priority": ["nixpkgs", "floco"]
 * }
 * ```
 */
struct RegistryRaw
{

  /** Settings and fetcher information associated with named inputs. */
  std::map<std::string, RegistryInput> inputs;

  /**
   * Priority order used to process inputs.
   * Inputs which do not appear in this list are handled in lexicographical
   * order after any explicitly named inputs.
   */
  std::vector<std::string> priority;


  /* Base class boilerplate. */
  virtual ~RegistryRaw()             = default;
  RegistryRaw()                      = default;
  RegistryRaw( const RegistryRaw & ) = default;
  RegistryRaw( RegistryRaw && )      = default;

  RegistryRaw &
  operator=( const RegistryRaw & )
    = default;
  RegistryRaw &
  operator=( RegistryRaw && )
    = default;

  explicit RegistryRaw( std::map<std::string, RegistryInput> inputs,
                        std::vector<std::string>             priority = {} )
    : inputs( std::move( inputs ) ), priority( std::move( priority ) )
  {}

  /**
   * @brief Return an ordered list of input names.
   *
   * This appends @a priority with any missing @a inputs in
   * lexicographical order.
   *
   * The resulting list contains wrapped references and need to be accessed
   * using @a std::reference_wrapper<T>::get().
   *
   * Example:
   * ```
   * Registry reg = R"( {
   *   "inputs": {
   *     "floco": {
   *       "from": { "type": "github", "owner": "aakropotkin", "repo": "floco" }
   *     }
   *   , "nixpkgs": {
   *       "from": { "type": "github", "owner": "NixOS", "repo": "nixpkgs" }
   *     }
   *   }
   * , "priority": ["nixpkgs"]
   * } )"_json;
   * for ( const auto & name : reg.getOrder() )
   *   {
   *     std::cout << name.get() << " ";
   *   }
   * std::cout << std::endl;
   * // => nixpkgs floco
   * ```
   *
   * @return A list of input names in order of priority.
   */
  [[nodiscard]] virtual std::vector<std::reference_wrapper<const std::string>>
  getOrder() const;

  /** @brief Reset to default state. */
  virtual void
  clear();

  /**
   * @brief Merge this @a flox::RegistryRaw struct with another
   *        @a flox::RegistryRaw, overriding any existing values with those from
   *        the other RegistryRaw
   *
   */
  void
  merge( const RegistryRaw & overrides );

  [[nodiscard]] bool
  operator==( const RegistryRaw & other ) const;

  [[nodiscard]] bool
  operator!=( const RegistryRaw & other ) const
  {
    return ! ( *this == other );
  }


}; /* End struct `RegistryRaw' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::RegistryRaw. */
void
from_json( const nlohmann::json & jfrom, RegistryRaw & reg );

/** @brief Convert a @a flox::RegistryRaw to a JSON object. */
void
to_json( nlohmann::json & jto, const RegistryRaw & reg );

/* -------------------------------------------------------------------------- */

/**
 * @class flox::InvalidRegistryException
 * @brief An exception thrown when a registry has invalid contents.
 * @{
 */
FLOX_DEFINE_EXCEPTION( InvalidRegistryException,
                       EC_INVALID_REGISTRY,
                       "invalid registry" )
/** @} */


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
