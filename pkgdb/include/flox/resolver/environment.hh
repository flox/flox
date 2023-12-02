/* ========================================================================== *
 *
 * @file flox/resolver/environment.hh
 *
 * @brief A collection of files associated with an environment.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <memory>
#include <optional>
#include <string_view>
#include <utility>
#include <vector>

#include <nix/ref.hh>

#include "flox/core/exceptions.hh"
#include "flox/core/nix-state.hh"
#include "flox/core/types.hh"
#include "flox/pkgdb/input.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/registry.hh"
#include "flox/resolver/lockfile.hh"
#include "flox/resolver/manifest-raw.hh"
#include "flox/resolver/manifest.hh"


/* -------------------------------------------------------------------------- */

/* Forward Declarations */

namespace flox {

namespace pkgdb {
class PkgDbReadOnly;
}  // namespace pkgdb

namespace resolver {
struct ManifestDescriptor;
}  // namespace resolver


}  // namespace flox


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

FLOX_DEFINE_EXCEPTION( ResolutionFailureException,
                       EC_RESOLUTION_FAILURE,
                       "resolution failure" );


/* -------------------------------------------------------------------------- */

/**
 * @brief A pair of _install ID_ and locked flake URLs used to record failed
 *        resolution attempts for a given descriptor.
 *
 * This allows us to more easily format exception messages.
 */
using ResolutionFailure = std::vector<std::pair<InstallID, std::string>>;

/**
 * @brief Either a set of resolved packages ( for a given system ) or a memo
 *        indicating that resolution failed for certain descriptors against
 *        certain inputs.
 *
 * When attempting to resolve a group of packages for a given system,
 * we either succeed and return @a flox::resolver::SystemPackages or
 * fail and return @a flox::resolver::ResolutionFailure.
 * This allows us to print descriptors that failed as groups for a
 * given input+rev.
 */
using ResolutionResult = std::variant<ResolutionFailure, SystemPackages>;


/* -------------------------------------------------------------------------- */

/**
 * @brief A collection of data associated with an environment and its state.
 *
 * This structure provides a number of helper routines which require knowledge
 * of manifests and lockfiles together - most importantly, locking descriptors.
 *
 * @see flox::resolver::GlobalManifest
 * @see flox::resolver::Manifest
 * @see flox::resolver::Lockfile
 */
class Environment : private NixStoreMixin
{

private:

  /* From `NixStoreMixin':
   *   std::shared_ptr<nix::Store> store
   */

  /** Contents of user level manifest with global registry and settings. */
  std::optional<GlobalManifest> globalManifest;

  /** The environment manifest. */
  EnvironmentManifest manifest;

  /** Previous generation of the lockfile ( if any ). */
  std::optional<Lockfile> oldLockfile;


  /**
   * @brief Indicator for lockfile upgrade operations.
   *
   * `true` means upgrade everything.
   * `false` or an empty vector mean upgrade nothing.
   * A list of `InstallID`s indicates a subset of packages to be upgraded.
   */
  using Upgrades = std::variant<bool, std::vector<InstallID>>;
  /** Packages to force an upgrade for, even if they are already locked. */
  Upgrades upgrades;

  /** New/modified lockfile being edited. */
  std::optional<LockfileRaw> lockfileRaw;

  std::optional<RegistryRaw> combinedRegistryRaw;

  std::optional<Options> combinedOptions;

  std::optional<pkgdb::PkgQueryArgs> combinedBaseQueryArgs;

  /** A registry of locked inputs. */
  std::optional<RegistryRaw> lockedRegistry;

  std::shared_ptr<Registry<pkgdb::PkgDbInputFactory>> dbs;


  static LockedPackageRaw
  lockPackage( const LockedInputRaw & input,
               pkgdb::PkgDbReadOnly & dbRO,
               pkgdb::row_id          row,
               unsigned               priority );

  static inline LockedPackageRaw
  lockPackage( const pkgdb::PkgDbInput & input,
               pkgdb::row_id             row,
               unsigned                  priority )
  {
    return lockPackage( LockedInputRaw( input ),
                        *input.getDbReadOnly(),
                        row,
                        priority );
  }

  /**
   * @brief Get groups that need to be locked as opposed to reusing locks from
   *        @a oldLockfile.
   */
  [[nodiscard]] std::vector<InstallDescriptors>
  getUnlockedGroups( const System & system );

  /** @brief Get groups with locks that can be reused from @a oldLockfile. */
  [[nodiscard]] std::vector<InstallDescriptors>
  getLockedGroups( const System & system );

  /**
   * @brief Get a merged form of @a oldLockfile or @a globalManifest
   *        ( if available ) and @a manifest options.
   *
   * Global options have the lowest priority, and will be clobbered by
   * locked options.
   * Options defined in the current manifest have the highest priority and will
   * clobber all other settings.
   */
  [[nodiscard]] const Options &
  getCombinedOptions();

  /** @brief Try to resolve a descriptor in a given package database. */
  [[nodiscard]] std::optional<pkgdb::row_id>
  tryResolveDescriptorIn( const ManifestDescriptor & descriptor,
                          const pkgdb::PkgDbInput &  input,
                          const System &             system );

  /**
   * @brief Try to resolve a group of descriptors
   *
   * Attempts to resolve using a locked input from the old lockfile if it exists
   * for the group. If not, inputs from the combined environment registry
   * are used.
   *
   * @return `std::nullopt` if resolution fails, otherwise a set of
   *          resolved packages.
   */
  [[nodiscard]] ResolutionResult
  tryResolveGroup( const InstallDescriptors & group, const System & system );

  /**
   * @brief Try to resolve a group of descriptors in a given package database.
   *
   * @return InstallID of the package that can't be resolved if resolution
   *         fails, otherwise a set of resolved packages for the system.
   */
  [[nodiscard]] std::variant<InstallID, SystemPackages>
  tryResolveGroupIn( const InstallDescriptors & group,
                     const pkgdb::PkgDbInput &  input,
                     const System &             system );

  /**
   * @brief Lock all descriptors for a given system.
   *        This is a helper function of
   *        @a flox::resolver::Environment::createLockfile().
   *
   * This must be called after @a lockfileRaw is initialized.
   * This is only intended to be called from
   * @a flox::resolver::Environment::createLockfile().
   */
  void
  lockSystem( const System & system );


protected:

  /**
   * @brief Get locked input from a lockfile to try to use to resolve a group
   *        of packages.
   *
   * Helper function for @a flox::resolver::Environment::lockSystem.
   * Choosing the locked input for a group is full of edge cases, because the
   * new group may be different than whatever was in the group in the
   * old lockfile.
   * We still want to reuse old locked inputs when we can.
   * For example:
   * - If the group name has changed, but nothing else has, we want to use the
   *   locked input.
   * - If packages have been added to a group, we want to use the locked input
   *   from a package that was already in the group.
   * - If groups are combined into a new group with a new name, we want to try
   *   to use one of the old locked inputs ( for now we just use the first one
   *   we find ).
   *
   * If, on the other hand, a package has changed, we don't want to use its
   * locked input.
   *
   * @return a locked input related to the group if we can find one,
   *         otherwise `std::nullopt`.
   */
  [[nodiscard]] std::optional<LockedInputRaw>
  getGroupInput( const InstallDescriptors & group,
                 const Lockfile &           oldLockfile,
                 const System &             system ) const;

  /**
   * @brief Check if lock from @ oldLockfile can be reused for a group.
   *
   * Checks if:
   * - All descriptors are present in the old manifest.
   * - No descriptors have changed in the old manifest such that the lock
   *   is invalidated.
   * - All descriptors are present in the old lock
   */
  [[nodiscard]] bool
  groupIsLocked( const InstallDescriptors & group,
                 const Lockfile &           oldLockfile,
                 const System &             system ) const;


public:

  Environment( std::optional<GlobalManifest> globalManifest,
               EnvironmentManifest           manifest,
               std::optional<Lockfile>       oldLockfile,
               Upgrades                      upgrades = false )
    : globalManifest( std::move( globalManifest ) )
    , manifest( std::move( manifest ) )
    , oldLockfile( std::move( oldLockfile ) )
    , upgrades( std::move( upgrades ) )
  {}

  explicit Environment( EnvironmentManifest     manifest,
                        std::optional<Lockfile> oldLockfile = std::nullopt )
    : globalManifest( std::nullopt )
    , manifest( std::move( manifest ) )
    , oldLockfile( std::move( oldLockfile ) )
  {}

  [[nodiscard]] const std::optional<GlobalManifest> &
  getGlobalManifest() const
  {
    return this->globalManifest;
  }

  [[nodiscard]] std::optional<GlobalManifestRaw>
  getGlobalManifestRaw() const
  {
    const auto & global = this->getGlobalManifest();
    if ( ! global.has_value() ) { return std::nullopt; }
    return global->getManifestRaw();
  }

  [[nodiscard]] const EnvironmentManifest &
  getManifest() const
  {
    return this->manifest;
  }

  [[nodiscard]] const ManifestRaw &
  getManifestRaw() const
  {
    return this->getManifest().getManifestRaw();
  }

  /** @brief Get the old manifest from @a oldLockfile if it exists. */
  [[nodiscard]] std::optional<ManifestRaw>
  getOldManifestRaw() const;

  [[nodiscard]] std::optional<Lockfile>
  getOldLockfile() const
  {
    return this->oldLockfile;
  }

  /**
   * @brief Get a merged form of @a oldLockfile ( if available ),
   *        @a globalManifest ( if available ) and @a manifest registries.
   *
   * The Global registry has the lowest priority, and will be clobbered by
   * locked registry inputs/settings.
   * The registry defined in the current manifest has the highest priority and
   * will clobber all other inputs/settings.
   */
  [[nodiscard]] RegistryRaw &
  getCombinedRegistryRaw();

  /**
   * @brief Get a base set of @a flox::pkgdb::PkgQueryArgs from
   *        combined options.
   */
  [[nodiscard]] const pkgdb::PkgQueryArgs &
  getCombinedBaseQueryArgs();

  /** @brief Get the set of supported systems. */
  [[nodiscard]] std::vector<System>
  getSystems() const
  {
    return this->getManifest().getSystems();
  }

  /** @brief Lazily initialize and get the combined registry's DBs. */
  [[nodiscard]] nix::ref<Registry<pkgdb::PkgDbInputFactory>>
  getPkgDbRegistry();

  // TODO: (Question) Should we lock the combined options and fill registry
  //                  `default` fields in inputs?
  /** @brief Create a new lockfile from @a manifest. */
  [[nodiscard]] Lockfile
  createLockfile();


}; /* End class `Environment' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
