/* ========================================================================== *
 *
 * @file flox/resolver/lockfile.hh
 *
 * @brief A lockfile representing a resolved environment.
 *
 * This lockfile is processed by `mkEnv` to realize an environment.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <unordered_map>

#include <nlohmann/json.hpp>

#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"
#include "flox/registry.hh"
#include "flox/resolver/manifest-raw.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

/**
 * @class flox::resolver::InvalidLockfileException
 * @brief An exception thrown when a lockfile is invalid.
 * @{
 */
FLOX_DEFINE_EXCEPTION( InvalidLockfileException,
                       EC_INVALID_LOCKFILE,
                       "invalid lockfile" )
/** @} */

/**
 * @class flox::resolver::PackageCheckFailure
 * @brief An exception thrown when a lockfile is invalid.
 * @{
 */
FLOX_DEFINE_EXCEPTION( PackageCheckFailure,
                       EC_PACKAGE_CHECK_FAILURE,
                       "bad package" )
/** @} */


/* -------------------------------------------------------------------------- */

struct LockedInputRaw
{

  std::string url; /**< Locked URI string.  */
  /** Exploded form of URI as an attr-set. */
  nlohmann::json attrs;

  LockedInputRaw()                         = default;
  ~LockedInputRaw()                        = default;
  LockedInputRaw( const LockedInputRaw & ) = default;
  LockedInputRaw( LockedInputRaw && )      = default;

  LockedInputRaw &
  operator=( const LockedInputRaw & )
    = default;
  LockedInputRaw &
  operator=( LockedInputRaw && )
    = default;

  explicit operator nix::FlakeRef() const
  {
    flox::NixState           nixState;
    nix::ref<nix::EvalState> state = nixState.getState();
    return nix::FlakeRef::fromAttrs(
      state->fetchSettings,
      nix::fetchers::jsonToAttrs( this->attrs ) );
  }

  explicit operator RegistryInput() const
  {
    return RegistryInput( static_cast<nix::FlakeRef>( *this ) );
  }

  [[nodiscard]] bool
  operator==( const LockedInputRaw & other ) const
  {
    return ( this->url == other.url ) && ( this->attrs == other.attrs );
  }

  [[nodiscard]] bool
  operator!=( const LockedInputRaw & other ) const
  {
    return ! ( ( *this ) == other );
  }


}; /* End struct `LockedInputRaw' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::resolver::LockedInputRaw. */
void
from_json( const nlohmann::json & jfrom, LockedInputRaw & raw );

/** @brief Convert a @a flox::resolver::LockedInputRaw to a JSON object. */
void
to_json( nlohmann::json & jto, const LockedInputRaw & raw );


/* -------------------------------------------------------------------------- */

/** @brief Print a locked input's to an output stream as a JSON object. */
std::ostream &
operator<<( std::ostream & oss, const LockedInputRaw & raw );


/* -------------------------------------------------------------------------- */


struct CheckPackageWarning
{
  std::string packageId;
  std::string message;
};

void
to_json( nlohmann::json & jto, const CheckPackageWarning & result );


/* -------------------------------------------------------------------------- */

/** @brief A locked package's _installable URI_. */
struct LockedPackageRaw
{

  LockedInputRaw input;
  AttrPath       attrPath;
  unsigned       priority;
  nlohmann::json info; /* pname, version, license */

  [[nodiscard]] bool
  operator==( const LockedPackageRaw & other ) const
  {
    return ( this->input == other.input )
           && ( this->attrPath == other.attrPath )
           && ( this->priority == other.priority )
           && ( this->info == other.info );
  }

  [[nodiscard]] bool
  operator!=( const LockedPackageRaw & other ) const
  {
    return ! ( ( *this ) == other );
  }


  [[nodiscard]] std::vector<CheckPackageWarning>
  check( const std::string &               packageId,
         const resolver::Options::Allows & allows ) const;
}; /* End struct `LockedPackageRaw' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::resolver::LockedPackageRaw. */
void
from_json( const nlohmann::json & jfrom, LockedPackageRaw & raw );

/** @brief Convert a @a flox::resolver::LockedPackageRaw to a JSON object. */
void
to_json( nlohmann::json & jto, const LockedPackageRaw & raw );


/* -------------------------------------------------------------------------- */

/** @brief Print a locked package to an output stream as a JSON object. */
std::ostream &
operator<<( std::ostream & oss, const LockedPackageRaw & raw );


/* -------------------------------------------------------------------------- */

using SystemPackages
  = std::unordered_map<InstallID, std::optional<LockedPackageRaw>>;

/**
 * @brief A v0 environment lockfile in its _raw_ form.
 */
struct LockfileRaw
{

  ManifestRaw                                manifest;
  RegistryRaw                                registry;
  std::unordered_map<System, SystemPackages> packages;
  unsigned                                   lockfileVersion = 0;


  ~LockfileRaw()                     = default;
  LockfileRaw()                      = default;
  LockfileRaw( const LockfileRaw & ) = default;
  // NOLINTNEXTLINE(bugprone-exception-escape)
  LockfileRaw( LockfileRaw && ) = default;
  LockfileRaw &
  operator=( const LockfileRaw & )
    = default;
  LockfileRaw &
  operator=( LockfileRaw && )
    = default;

  /**
   * @brief Check the lockfile for validity, throw and exception if it
   *        is invalid.
   *
   * This checks that:
   * - The lockfile version is supported.
   */
  void
  check() const;

  /** @brief Reset to default/empty state. */
  void
  clear();
}; /* End struct `LockfileRaw' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::resolver::LockfileRaw. */
void
from_json( const nlohmann::json & jfrom, LockfileRaw & raw );

/** @brief Convert a @a flox::resolver::LockfileRaw to a JSON object. */
void
to_json( nlohmann::json & jto, const LockfileRaw & raw );

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
