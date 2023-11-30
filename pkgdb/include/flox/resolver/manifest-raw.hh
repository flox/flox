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

#include "compat/concepts.hh"
#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/registry.hh"
#include "flox/resolver/descriptor.hh"  // IWYU pragma: keep


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

/* Forward Declarations */

struct GlobalManifestRaw;
struct ManifestRaw;
struct GlobalManifestRawGA;
struct ManifestRawGA;


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

  struct Semver
  {
    std::optional<bool> preferPreReleases;
  }; /* End struct `Semver' */
  std::optional<Semver> semver;

  std::optional<std::string> packageGroupingStrategy;
  std::optional<std::string> activationStrategy;
  // TODO: Other options


  /**
   * @brief Apply options from @a overrides, but retain other existing options.
   */
  void
  merge( const Options & overrides );

  /** @brief Convert to a _base_ set of @a flox::pkgdb::PkgQueryArgs. */
  explicit operator pkgdb::PkgQueryArgs() const;


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

  explicit operator GlobalManifestRawGA() const;


}; /* End struct `GlobalManifestRaw' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::resolver::GlobalManifestRaw. */
void
from_json( const nlohmann::json & jfrom, GlobalManifestRaw & manifest );

/** @brief Convert a @a flox::resolver::GlobalManifestRaw to a JSON object. */
void
to_json( nlohmann::json & jto, const GlobalManifestRaw & manifest );


/* -------------------------------------------------------------------------- */

/** @brief Declares a base environment to extend. */
struct EnvBaseRaw
{
  /** Indicates a remote URL to be extended. */
  std::optional<std::string> floxhub;

  /**
   * Indicates a local directory with a `.flox/` subdirectory to be extended.
   */
  std::optional<std::string> dir;


  /**
   * @brief Validate the `env-base` field, throwing an exception if invalid
   *        information is found.
   *
   * This asserts:
   * - Only one of `floxhub` or `dir` is set.
   */
  void
  check() const;

  void
  clear()
  {
    this->floxhub = std::nullopt;
    this->dir     = std::nullopt;
  }


}; /* End struct `EnvBaseRaw' */


/* -------------------------------------------------------------------------- */

/** @brief Declares a hook to be run at environment activation. */
struct HookRaw
{
  /** Define an inline script to be run at activation time. */
  std::optional<std::string> script;

  /** Reads activation script from a file. */
  std::optional<std::string> file;


  /**
   * @brief Validate `Hook` fields, throwing an exception if its contents
   *        are invalid.
   */
  void
  check() const;


}; /* End struct `HookRaw' */


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

  std::optional<EnvBaseRaw> envBase;

  std::optional<
    std::unordered_map<InstallID, std::optional<ManifestDescriptorRaw>>>
    install;

  std::optional<std::unordered_map<std::string, std::string>> vars;

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
   * - @a envBase is valid.
   * - @a registry does not contain indirect flake references.
   * - All members of @a install are valid.
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
    this->envBase = std::nullopt;
    this->install = std::nullopt;
    this->vars    = std::nullopt;
    this->hook    = std::nullopt;
  }

  /**
   * @brief Generate a JSON _diff_ between @a this manifest an @a old manifest.
   *
   * The _diff_ is represented as an [JSON patch](https://jsonpatch.com) object.
   */
  nlohmann::json
  diff( const ManifestRaw & old ) const;

  explicit operator ManifestRawGA() const;


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
 * @brief A _global_ manifest containing only `registry` and `options` fields
 *        in its _raw_ form.
 *        This form is limited to only the `options` field
 *        ( dropping `registry` ) for use with `flox`'s GA release.
 *
 * This _raw_ struct is defined to generate parsers, and its declarations simply
 * represent what is considered _valid_.
 * On its own, it performs no real work, other than to validate the input.
 *
 * @see flox::resolver::GlobalManifestGA
 */
struct GlobalManifestRawGA
{

  /** @brief Options controlling environment and search behaviors. */
  std::optional<Options> options;


  virtual ~GlobalManifestRawGA()                     = default;
  GlobalManifestRawGA()                              = default;
  GlobalManifestRawGA( const GlobalManifestRawGA & ) = default;
  GlobalManifestRawGA( GlobalManifestRawGA && )      = default;

  explicit GlobalManifestRawGA( std::optional<Options> options )
    : options( std::move( options ) )
  {}

  GlobalManifestRawGA &
  operator=( const GlobalManifestRawGA & )
    = default;

  GlobalManifestRawGA &
  operator=( GlobalManifestRawGA && )
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
    this->options = std::nullopt;
  }

  explicit operator GlobalManifestRaw() const
  {
    return GlobalManifestRaw( getGARegistry(), this->options );
  }

  explicit operator ManifestRaw() const
  {
    return ManifestRaw( static_cast<GlobalManifestRaw>( *this ) );
  }


}; /* End struct `GlobalManifestRawGA' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::resolver::GlobalManifestRawGA. */
void
from_json( const nlohmann::json & jfrom, GlobalManifestRawGA & manifest );

/** @brief Convert a @a flox::resolver::GlobalManifestRawGA to a JSON object. */
void
to_json( nlohmann::json & jto, const GlobalManifestRawGA & manifest );


/* -------------------------------------------------------------------------- */

/**
 * @brief A _raw_ description of an environment to be read from a file.
 *        This form drops the `registry` field for use with `flox`'s GA release.
 *
 * This _raw_ struct is defined to generate parsers, and its declarations simply
 * represent what is considered _valid_.
 * On its own, it performs no real work, other than to validate the input.
 *
 * @see flox::resolver::ManifestGA
 */
struct ManifestRawGA : public GlobalManifestRawGA
{

  std::optional<
    std::unordered_map<InstallID, std::optional<ManifestDescriptorRaw>>>
    install;

  std::optional<std::unordered_map<std::string, std::string>> vars;

  std::optional<HookRaw> hook;


  ~ManifestRawGA() override              = default;
  ManifestRawGA()                        = default;
  ManifestRawGA( const ManifestRawGA & ) = default;
  ManifestRawGA( ManifestRawGA && )      = default;

  explicit ManifestRawGA( const GlobalManifestRawGA & globalManifestRawGA )
    : GlobalManifestRawGA( globalManifestRawGA )
  {}

  explicit ManifestRawGA( GlobalManifestRawGA && globalManifestRawGA )
    : GlobalManifestRawGA( globalManifestRawGA )
  {}

  ManifestRawGA &
  operator=( const ManifestRawGA & )
    = default;

  ManifestRawGA &
  operator=( ManifestRawGA && )
    = default;

  ManifestRawGA &
  operator=( const GlobalManifestRawGA & globalManifestRawGA )
  {
    GlobalManifestRawGA::operator=( globalManifestRawGA );
    return *this;
  }

  ManifestRawGA &
  operator=( GlobalManifestRawGA && globalManifestRawGA )
  {
    GlobalManifestRawGA::operator=( globalManifestRawGA );
    return *this;
  }

  /**
   * @brief Validate manifest fields, throwing an exception if its contents
   *        are invalid.
   *
   * This asserts:
   * - All members of @a install are valid.
   * - @a hook is valid.
   */
  void
  check() const override;

  void
  clear() override
  {
    /* From `GlobalManifestRawGA' */
    this->options = std::nullopt;
    /* From `ManifestRawGA' */
    this->install = std::nullopt;
    this->vars    = std::nullopt;
    this->hook    = std::nullopt;
  }

  /**
   * @brief Generate a JSON _diff_ between @a this manifest an @a old manifest.
   *
   * The _diff_ is represented as an [JSON patch](https://jsonpatch.com) object.
   */
  nlohmann::json
  diff( const ManifestRawGA & old ) const;

  explicit operator ManifestRaw() const
  {
    ManifestRaw raw;
    raw.registry = getGARegistry();
    raw.options  = this->options;
    raw.install  = this->install;
    raw.vars     = this->vars;
    raw.hook     = this->hook;
    return raw;
  }


}; /* End struct `ManifestRawGA' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::resolver::ManifestRawGA. */
void
from_json( const nlohmann::json & jfrom, ManifestRawGA & manifest );

/** @brief Convert a @a flox::resolver::ManifestRawGA to a JSON object. */
void
to_json( nlohmann::json & jto, const ManifestRawGA & manifest );


/* -------------------------------------------------------------------------- */

/**
 * @brief Restrict types to those derived from
 *        @a flox::resolver::GlobalManifestRaw or
 *        @a flox::resolver::GlobalManifestRawGA. */
template<typename RawType>
concept manifest_raw_type = std::derived_from<RawType, GlobalManifestRaw>
                            || std::derived_from<RawType, GlobalManifestRawGA>;

static_assert( manifest_raw_type<GlobalManifestRaw> );
static_assert( manifest_raw_type<GlobalManifestRawGA> );
static_assert( manifest_raw_type<ManifestRaw> );
static_assert( manifest_raw_type<ManifestRawGA> );


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
