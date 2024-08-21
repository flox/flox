/* ========================================================================== *
 *
 * @file flox/resolver/manifest-raw.hh
 *
 * @brief An abstract description of an environment in its unresolved state.
 *        This representation is intended for serialization and deserialization.
 *        For the _real_ representation, see
 *        [flox/resolver/manifest.hh](./manifest.hh).
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <algorithm>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

#include <nlohmann/json.hpp>

#include "flox/core/exceptions.hh"
#include "flox/core/nix-state.hh"
#include "flox/core/types.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

/* Forward Declarations */

struct GlobalManifestRaw;
struct ManifestRaw;


/* -------------------------------------------------------------------------- */

/**
 * @class flox::resolver::InvalidManifestFileException
 * @brief An exception thrown when a manifest file is invalid.
 * @{
 */
FLOX_DEFINE_EXCEPTION( InvalidManifestFileException,
                       EC_INVALID_MANIFEST_FILE,
                       "invalid manifest file" )
/** @} */


/* -------------------------------------------------------------------------- */

/**
 * @brief The `install.<INSTALL-ID>` field name associated with a package
 *        or descriptor.
 */
using InstallID = std::string;


/* -------------------------------------------------------------------------- */

/** @brief A set of options that apply to an entire environment. */
struct Options
{

  std::optional<std::vector<System>> systems;

  struct Allows
  {
    std::optional<bool>                     unfree;
    std::optional<bool>                     broken;
    std::optional<std::vector<std::string>> licenses;
  }; /* End struct `Allows' */
  std::optional<Allows> allow;

}; /* End struct `Options' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::resolver::Options. */
void
from_json( const nlohmann::json & jfrom, Options & opts );

/** @brief Convert a @a flox::resolver::Options to a JSON Object. */
void
to_json( nlohmann::json & jto, const Options & opts );


/* -------------------------------------------------------------------------- */

/**
 * @brief A _global_ manifest containing only `registry` and `options` fields
 *        in its _raw_ form.
 *
 * This _raw_ struct is defined to generate parsers, and its declarations simply
 * represent what is considered _valid_.
 * On its own, it performs no real work, other than to validate the input.
 *
 * @see flox::resolver::GlobalManifest
 */
struct GlobalManifestRaw
{
  /** A collection of _inputs_ to find packages. */
  std::optional<RegistryRaw> registry;

  /** @brief Options controlling environment and search behaviors. */
  std::optional<Options> options;


  virtual ~GlobalManifestRaw()                   = default;
  GlobalManifestRaw()                            = default;
  GlobalManifestRaw( const GlobalManifestRaw & ) = default;
  GlobalManifestRaw( GlobalManifestRaw && )      = default;

  explicit GlobalManifestRaw( std::optional<RegistryRaw> registry,
                              std::optional<Options> options = std::nullopt )
    : registry( std::move( registry ) ), options( std::move( options ) )
  {}

  explicit GlobalManifestRaw( std::optional<Options> options )
    : options( std::move( options ) )
  {}

  GlobalManifestRaw &
  operator=( const GlobalManifestRaw & )
    = default;
  GlobalManifestRaw &
  operator=( GlobalManifestRaw && )
    = default;

  /**
   * @brief Validate manifest fields, throwing an exception if its contents
   *        are invalid.
   */
  virtual void
  check() const
  {}

  virtual void
  clear()
  {
    this->registry = std::nullopt;
    this->options  = std::nullopt;
  }

  /**
   * @brief Get the list of systems requested by the manifest.
   *
   * Default to the current system if systems is not specified.
   */
  [[nodiscard]] std::vector<System>
  getSystems() const
  {
    if ( this->options.has_value() && this->options->systems.has_value() )
      {
        return *this->options->systems;
      }
    return std::vector<System> { nix::settings.thisSystem.get() };
  }
}; /* End struct `GlobalManifestRaw' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::resolver::GlobalManifestRaw. */
void
from_json( const nlohmann::json & jfrom, GlobalManifestRaw & manifest );

/** @brief Convert a @a flox::resolver::GlobalManifestRaw to a JSON object. */
void
to_json( nlohmann::json & jto, const GlobalManifestRaw & manifest );

/* -------------------------------------------------------------------------- */

/** @brief Declares a hook to be run at environment activation. */
struct HookRaw
{
  /** Define an inline script to be run at activation time. */
  std::optional<std::string> script;

  /** Defines an inline script to be run non-interactively from a bash subshell
   * after the user's profile scripts have been sourced.*/
  std::optional<std::string> onActivate;


  /**
   * @brief Validate `Hook` fields, throwing an exception if its contents
   *        are invalid.
   */
  void
  check() const;


}; /* End struct `HookRaw' */

void
from_json( const nlohmann::json & jfrom, HookRaw & hook );

/* -------------------------------------------------------------------------- */

/** @brief Declares scripts to be sourced by the user's interactive shell after
 * activating the environment.*/
struct ProfileScriptsRaw
{
  /** @brief A script intended to be sourced by all shells. */
  std::optional<std::string> common;

  /** @brief A script intended to be sourced only in Bash shells. */
  std::optional<std::string> bash;

  /** @brief A script intended to be sourced only in Fish shells. */
  std::optional<std::string> fish;

  /** @brief A script intended to be sourced only in Tcsh shells. */
  std::optional<std::string> tcsh;

  /** @brief A script intended to be sourced only in Zsh shells. */
  std::optional<std::string> zsh;
};

void
from_json( const nlohmann::json & jfrom, ProfileScriptsRaw & profile );


/* -------------------------------------------------------------------------- */

/**
 * @brief A _raw_ description of an environment to be read from a file.
 *
 * This _raw_ struct is defined to generate parsers, and its declarations simply
 * represent what is considered _valid_.
 * On its own, it performs no real work, other than to validate the input.
 *
 * @see flox::resolver::Manifest
 */
struct ManifestRaw : public GlobalManifestRaw
{
  std::optional<std::unordered_map<std::string, std::string>> vars;

  std::optional<ProfileScriptsRaw> profile;

  std::optional<HookRaw> hook;


  ~ManifestRaw() override            = default;
  ManifestRaw()                      = default;
  ManifestRaw( const ManifestRaw & ) = default;
  ManifestRaw( ManifestRaw && )      = default;

  explicit ManifestRaw( const GlobalManifestRaw & globalManifestRaw )
    : GlobalManifestRaw( globalManifestRaw )
  {}

  explicit ManifestRaw( GlobalManifestRaw && globalManifestRaw )
    : GlobalManifestRaw( globalManifestRaw )
  {}

  ManifestRaw &
  operator=( const ManifestRaw & )
    = default;

  ManifestRaw &
  operator=( ManifestRaw && )
    = default;

  ManifestRaw &
  operator=( const GlobalManifestRaw & globalManifestRaw )
  {
    GlobalManifestRaw::operator=( globalManifestRaw );
    return *this;
  }

  ManifestRaw &
  operator=( GlobalManifestRaw && globalManifestRaw )
  {
    GlobalManifestRaw::operator=( globalManifestRaw );
    return *this;
  }

  /**
   * @brief Validate manifest fields, throwing an exception if its contents
   *        are invalid.
   *
   * This asserts:
   * - @a registry does not contain indirect flake references.
   * - @a hook is valid.
   */
  void
  check() const override;

  void
  clear() override
  {
    /* From `GlobalManifestRaw' */
    this->options  = std::nullopt;
    this->registry = std::nullopt;
    /* From `ManifestRaw' */
    this->vars    = std::nullopt;
    this->hook    = std::nullopt;
    this->profile = std::nullopt;
  }

  /**
   * @brief Generate a JSON _diff_ between @a this manifest an @a old manifest.
   *
   * The _diff_ is represented as an [JSON patch](https://jsonpatch.com) object.
   */
  [[nodiscard]] nlohmann::json
  diff( const ManifestRaw & old ) const;

}; /* End struct `ManifestRaw' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::resolver::ManifestRaw. */
void
from_json( const nlohmann::json & jfrom, ManifestRaw & manifest );

/** @brief Convert a @a flox::resolver::ManifestRaw to a JSON object. */
void
to_json( nlohmann::json & jto, const ManifestRaw & manifest );

/* -------------------------------------------------------------------------- */

/**
 * @brief Restrict types to those derived from
 *        @a flox::resolver::GlobalManifestRaw or
 *        @a flox::resolver::GlobalManifestRawGA. */
template<typename RawType>
concept manifest_raw_type = std::derived_from<RawType, GlobalManifestRaw>;

static_assert( manifest_raw_type<GlobalManifestRaw> );
static_assert( manifest_raw_type<ManifestRaw> );


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
