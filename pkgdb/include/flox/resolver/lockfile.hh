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
#include "flox/pkgdb/input.hh"
#include "flox/pkgdb/read.hh"
#include "flox/registry.hh"
#include "flox/resolver/manifest.hh"


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


/* -------------------------------------------------------------------------- */

// XXX: Post-GA if we use non-nixpkgs inputs, or want to support user defined
//      _scrape rules_ we will need to add fields here to handle those.
//      For now we assume all inputs are nixpkgs and we use the `fingerprint`
//      field to track the _scrape rules_ wrapper.
//      The _actual_ `attrs` and `url` here will only align with the fingerprint
//      if the _scrape rules_ wrapper is used.
struct LockedInputRaw
{

  pkgdb::Fingerprint fingerprint; /**< Unique hash of associated flake. */
  std::string        url;         /**< Locked URI string.  */
  /** Exploded form of URI as an attr-set. */
  nlohmann::json attrs;

  ~LockedInputRaw() = default;
  LockedInputRaw() : fingerprint( nix::htSHA256 ) {}
  LockedInputRaw( const LockedInputRaw & ) = default;
  LockedInputRaw( LockedInputRaw && )      = default;

  LockedInputRaw &
  operator=( const LockedInputRaw & )
    = default;
  LockedInputRaw &
  operator=( LockedInputRaw && )
    = default;

  explicit LockedInputRaw( const pkgdb::PkgDbReadOnly & pdb )
    : fingerprint( pdb.fingerprint )
    , url( pdb.lockedRef.string )
    , attrs( pdb.lockedRef.attrs )
  {}

  explicit LockedInputRaw( const pkgdb::PkgDbInput & input )
    : LockedInputRaw( *input.getDbReadOnly() )
  {}

  explicit operator nix::FlakeRef() const
  {
    return nix::FlakeRef::fromAttrs(
      nix::fetchers::jsonToAttrs( this->attrs ) );
  }

  explicit operator RegistryInput() const
  {
    return RegistryInput( static_cast<nix::FlakeRef>( *this ) );
  }

  [[nodiscard]] bool
  operator==( const LockedInputRaw & other ) const
  {
    return ( this->fingerprint == other.fingerprint )
           && ( this->url == other.url ) && ( this->attrs == other.attrs );
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
 * @brief An environment lockfile in its _raw_ form.
 *
 * This form is suitable for _instantiating_ ( _i.e._, realizing ) an
 * environment using `mkEnv`.
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
  LockfileRaw( LockfileRaw && )      = default;
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


/* -------------------------------------------------------------------------- */

/**
 * @brief A locked representation of an environment.
 *
 * Unlike the _raw_ form, this form is suitable for stashing temporary variables
 * and other information that is not needed for serializing/de-serializing.
 */
class Lockfile
{

private:

  /** Raw representation of the lockfile. */
  LockfileRaw lockfileRaw;

  /**
   * Handle for the manifest used to create the lockfile.
   * This reads the lockfile's `manifest`.
   */
  EnvironmentManifest manifest;
  /** Maps `{ <FINGERPRINT>: <INPUT> }` for all `packages` members' inputs. */
  RegistryRaw packagesRegistryRaw;


  /**
   * @brief Check the lockfile's `packages.**` locked inputs align with the
   *        requested groups in `manifest.install.<INSTALL-ID>.packageGroup`,
   *        Throws an exception if two packages in the same group use
   *        different inputs.
   */
  void
  checkGroups() const;

  /**
   * @brief Check the lockfile's validity, throwing an exception for
   *        invalid contents.
   *
   * This asserts that:
   * - `lockfileVersion` is supported.
   * - `packages` members' groups are enforced.
   * - original _manifest_ is consistent with the lockfile's
   *   `registry.*` and `packages.**` members for `optional` and
   *   `systems` skipping.
   * - `registry` inputs do not use indirect flake references.
   */
  void
  check() const;

  /**
   * @brief Initialize @a manifest and @a packagesRegistryRaw from
   *        @a lockfileRaw.
   */
  void
  init();


public:

  ~Lockfile()                  = default;
  Lockfile()                   = default;
  Lockfile( const Lockfile & ) = default;
  Lockfile( Lockfile && )      = default;

  explicit Lockfile( LockfileRaw raw ) : lockfileRaw( std::move( raw ) )
  {
    this->init();
  }

  explicit Lockfile( std::filesystem::path lockfilePath );

  Lockfile &
  operator=( const Lockfile & )
    = default;

  Lockfile &
  operator=( Lockfile && )
    = default;

  /** @brief Get the _raw_ representation of the lockfile. */
  [[nodiscard]] const LockfileRaw &
  getLockfileRaw() const
  {
    return this->lockfileRaw;
  }

  /** @brief Get the original _manifest_ used to create the lockfile. */
  [[nodiscard]] const ManifestRaw &
  getManifestRaw() const
  {
    return this->getLockfileRaw().manifest;
  }

  /** @brief Get the locked registry from the _raw_ lockfile. */
  [[nodiscard]] const RegistryRaw &
  getRegistryRaw() const
  {
    return this->getLockfileRaw().registry;
  }

  /** @brief Get old manifest. */
  [[nodiscard]] const EnvironmentManifest &
  getManifest() const
  {
    return this->manifest;
  }

  /** @brief Get old descriptors. */
  [[nodiscard]] const std::unordered_map<InstallID, ManifestDescriptor> &
  getDescriptors() const
  {
    return this->getManifest().getDescriptors();
  }

  /**
   * @brief Get the @a packagesRegistryRaw, containing all inputs used by
   *        `packages.**` members of the lockfile.
   *
   * This registry keys inputs by their fingerprints.
   */
  [[nodiscard]] const RegistryRaw &
  getPackagesRegistryRaw() const
  {
    return this->packagesRegistryRaw;
  }

  /**
   * @brief Drop any `registry.inputs` and `registry.priority` members that are
   *        not explicitly declared in the manifest `registry` or used by
   *        resolved packages.
   *
   * @return The number of removed inputs.
   */
  std::size_t
  removeUnusedInputs();


}; /* End class `Lockfile' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
