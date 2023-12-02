/* ========================================================================== *
 *
 * @file resolver/environment.cc
 *
 * @brief A collection of files associated with an environment.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <assert.h>
#include <memory>
#include <optional>
#include <ostream>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

#include <nix/flake/flakeref.hh>
#include <nix/ref.hh>
#include <nlohmann/json.hpp>

#include "flox/core/types.hh"
#include "flox/pkgdb/input.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/pkgdb/read.hh"
#include "flox/registry.hh"
#include "flox/resolver/descriptor.hh"
#include "flox/resolver/environment.hh"
#include "flox/resolver/lockfile.hh"
#include "flox/resolver/manifest-raw.hh"
#include "flox/resolver/manifest.hh"


/* -------------------------------------------------------------------------- */

/* Forward Declarations */

namespace nix {
class Store;
}


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

RegistryRaw &
Environment::getCombinedRegistryRaw()
{
  if ( ! this->combinedRegistryRaw.has_value() )
    {
      /* Start with the global manifest's registry ( if any ), and merge it with
       * the environment manifest's registry. */
      if ( auto maybeGlobal = this->getGlobalManifest();
           maybeGlobal.has_value() )
        {
          this->combinedRegistryRaw = maybeGlobal->getLockedRegistry();
          this->combinedRegistryRaw->merge(
            this->getManifest().getLockedRegistry() );
        }
      else
        {
          this->combinedRegistryRaw = this->getManifest().getLockedRegistry();
        }

      /* If there's a lockfile, use pinned inputs.
       * However, do not preserve any inputs that were removed from
       * the manifest. */
      if ( auto maybeLock = this->getOldLockfile(); maybeLock.has_value() )
        {
          auto lockedRegistry = maybeLock->getRegistryRaw();
          for ( auto & [name, input] : this->combinedRegistryRaw->inputs )
            {
              /* Use the pinned input from the lock if it exists. */
              if ( auto locked = lockedRegistry.inputs.find( name );
                   locked != lockedRegistry.inputs.end() )
                {
                  input = locked->second;
                }
            }
        }
    }
  return *this->combinedRegistryRaw;
}


/* -------------------------------------------------------------------------- */

nix::ref<Registry<pkgdb::PkgDbInputFactory>>
Environment::getPkgDbRegistry()
{
  if ( this->dbs == nullptr )
    {
      nix::ref<nix::Store> store   = this->getStore();
      auto                 factory = pkgdb::PkgDbInputFactory( store );
      this->dbs = std::make_shared<Registry<pkgdb::PkgDbInputFactory>>(
        this->getCombinedRegistryRaw(),
        factory );
      /* Scrape if needed. */
      for ( auto & [name, input] : *this->dbs )
        {
          input->scrapeSystems( this->getSystems() );
        }
    }
  return static_cast<nix::ref<Registry<pkgdb::PkgDbInputFactory>>>( this->dbs );
}


/* -------------------------------------------------------------------------- */

std::optional<ManifestRaw>
Environment::getOldManifestRaw() const
{
  if ( this->getOldLockfile().has_value() )
    {
      return this->getOldLockfile()->getManifestRaw();
    }
  return std::nullopt;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Helper function for @a flox::resolver::Environment::groupIsLocked.
 *
 * A system is skipped if systems is specified but that system is not.
 */
[[nodiscard]] static bool
systemSkipped( const System &                             system,
               const std::optional<std::vector<System>> & systems )
{
  return systems.has_value()
         && ( std::find( systems->begin(), systems->end(), system )
              == systems->end() );
}


/* -------------------------------------------------------------------------- */

bool
Environment::groupIsLocked( const InstallDescriptors & group,
                            const Lockfile &           oldLockfile,
                            const System &             system ) const
{
  auto packages = oldLockfile.getLockfileRaw().packages;
  if ( ! packages.contains( system ) ) { return false; }

  SystemPackages oldSystemPackages = packages.at( system );

  InstallDescriptors oldDescriptors = oldLockfile.getDescriptors();

  /* Check for upgrades. */
  for ( auto & [iid, descriptor] : group )
    {
      /* Check for upgrades. */
      if ( const bool * upgradeEverything
           = std::get_if<bool>( &this->upgrades ) )
        {
          /* If we are upgrading everything, we treat all groups
           * as "unlocked". */
          if ( *upgradeEverything ) { return false; }
        }
      else
        {
          /* If the current iid is being upgraded, the group needs to be
           * locked again. */
          auto upgrades = std::get<std::vector<InstallID>>( this->upgrades );
          if ( std::find( upgrades.begin(), upgrades.end(), iid )
               != upgrades.end() )
            {
              return false;
            }
        }

      /* If the descriptor has changed compared to the one in the lockfile
       * manifest, it needs to be locked again. */
      if ( auto oldDescriptorPair = oldDescriptors.find( iid );
           oldDescriptorPair == oldDescriptors.end() )
        {
          /* If the descriptor doesn't even exist in the lockfile manifest, it
           * needs to be locked again. */
          return false;
        }
      else
        {
          auto & [_, oldDescriptor] = *oldDescriptorPair;

          /* We ignore `priority' and handle `systems' below. */
          if ( ( descriptor.name != oldDescriptor.name )
               || ( descriptor.path != oldDescriptor.path )
               || ( descriptor.version != oldDescriptor.version )
               || ( descriptor.semver != oldDescriptor.semver )
               || ( descriptor.subtree != oldDescriptor.subtree )
               || ( descriptor.input != oldDescriptor.input )
               || ( descriptor.group != oldDescriptor.group )
               || ( descriptor.optional != oldDescriptor.optional ) )
            {
              return false;
            }

          /* Ignore changes to systems other than the one we're locking. */
          if ( systemSkipped( system, descriptor.systems )
               != systemSkipped( system, oldDescriptor.systems ) )
            {
              return false;
            }
        }

      /* Check if the descriptor exists in the lockfile lock */
      if ( auto oldLockedPackagePair = oldSystemPackages.find( iid );
           oldLockedPackagePair == oldSystemPackages.end() )
        {
          /* If the descriptor doesn't even exist in the lockfile lock, it needs
           * to be locked again.
           * This should be unreachable since the descriptor shouldn't exist in
           * the lockfile manifest if it doesn't exist in the lockfile. */
          return false;
        }
      // else
      //   {
      //     /* NOTE: we could relock if the prior locking attempt was null */
      //     auto & [_, oldLockedPackage] = *oldLockedPackagePair;
      //     if ( !oldLockedPackage.has_value()) { return false; }
      //   }
    }

  /* We haven't found something unlocked, so everything must be locked. */
  return true;
}


/* -------------------------------------------------------------------------- */

std::vector<InstallDescriptors>
Environment::getUnlockedGroups( const System & system )
{
  auto lockfile           = this->getOldLockfile();
  auto groupedDescriptors = this->getManifest().getGroupedDescriptors();
  if ( ! lockfile.has_value() ) { return groupedDescriptors; }

  for ( auto group = groupedDescriptors.begin();
        group != groupedDescriptors.end(); )
    {
      if ( groupIsLocked( *group, *lockfile, system ) )
        {
          group = groupedDescriptors.erase( group );
        }
      else { ++group; }
    }

  return groupedDescriptors;
}


/* -------------------------------------------------------------------------- */

std::vector<InstallDescriptors>
Environment::getLockedGroups( const System & system )
{
  auto lockfile = this->getOldLockfile();
  if ( ! lockfile.has_value() ) { return std::vector<InstallDescriptors> {}; }

  auto groupedDescriptors = this->getManifest().getGroupedDescriptors();

  /* Remove all groups that are *not* already locked. */
  for ( auto group = groupedDescriptors.begin();
        group != groupedDescriptors.end(); )
    {
      if ( ! groupIsLocked( *group, *lockfile, system ) )
        {
          group = groupedDescriptors.erase( group );
        }
      else { ++group; }
    }

  return groupedDescriptors;
}


/* -------------------------------------------------------------------------- */

const Options &
Environment::getCombinedOptions()
{
  if ( ! this->combinedOptions.has_value() )
    {
      /* Start with global options ( if any ). */
      if ( this->getGlobalManifest().has_value()
           && this->getGlobalManifestRaw()->options.has_value() )
        {
          this->combinedOptions = this->getGlobalManifestRaw()->options;
        }
      else { this->combinedOptions = Options {}; }

      /* Clobber with lockfile's options ( if any ). */
      if ( this->getOldManifestRaw().has_value()
           && this->getOldManifestRaw()->options.has_value() )
        {
          this->combinedOptions->merge( *this->getOldManifestRaw()->options );
        }

      /* Clobber with manifest's options ( if any ). */
      if ( this->getManifestRaw().options.has_value() )
        {
          this->combinedOptions->merge( *this->getManifestRaw().options );
        }
    }
  return *this->combinedOptions;
}


/* -------------------------------------------------------------------------- */

const pkgdb::PkgQueryArgs &
Environment::getCombinedBaseQueryArgs()
{
  if ( ! this->combinedBaseQueryArgs.has_value() )
    {
      this->combinedBaseQueryArgs
        = static_cast<pkgdb::PkgQueryArgs>( this->getCombinedOptions() );
    }
  return *this->combinedBaseQueryArgs;
}


/* -------------------------------------------------------------------------- */

std::optional<pkgdb::row_id>
Environment::tryResolveDescriptorIn( const ManifestDescriptor & descriptor,
                                     const pkgdb::PkgDbInput &  input,
                                     const System &             system )
{
  /* Skip unrequested systems. */
  if ( descriptor.systems.has_value()
       && ( std::find( descriptor.systems->begin(),
                       descriptor.systems->end(),
                       system )
            == descriptor.systems->end() ) )
    {
      return std::nullopt;
    }

  pkgdb::PkgQueryArgs args = this->getCombinedBaseQueryArgs();
  input.fillPkgQueryArgs( args );
  descriptor.fillPkgQueryArgs( args );
  /* Limit results to the target system. */
  args.systems = std::vector<System> { system };
  pkgdb::PkgQuery query( args );
  auto            rows = query.execute( input.getDbReadOnly()->db );
  if ( rows.empty() ) { return std::nullopt; }
  return rows.front();
}


/* -------------------------------------------------------------------------- */

LockedPackageRaw
Environment::lockPackage( const LockedInputRaw & input,
                          pkgdb::PkgDbReadOnly & dbRO,
                          pkgdb::row_id          row,
                          unsigned               priority )
{
  nlohmann::json   info = dbRO.getPackage( row );
  LockedPackageRaw pkg;
  pkg.input = input;
  info.at( "absPath" ).get_to( pkg.attrPath );
  info.erase( "id" );
  info.erase( "description" );
  info.erase( "absPath" );
  info.erase( "subtree" );
  info.erase( "system" );
  info.erase( "relPath" );
  pkg.priority = priority;
  pkg.info     = std::move( info );
  return pkg;
}


/* -------------------------------------------------------------------------- */

std::optional<LockedInputRaw>
Environment::getGroupInput( const InstallDescriptors & group,
                            const Lockfile &           oldLockfile,
                            const System &             system ) const
{
  auto packages = oldLockfile.getLockfileRaw().packages;
  if ( ! packages.contains( system ) ) { return std::nullopt; }
  SystemPackages oldSystemPackages = packages.at( system );

  InstallDescriptors oldDescriptors = oldLockfile.getDescriptors();

  std::optional<LockedInputRaw> wrongGroupInput;
  /* We could look for packages where just the _iid_ has changed, but for now
   * just use _iid_. */
  for ( const auto & [iid, descriptor] : group )
    {
      if ( auto it = oldSystemPackages.find( iid );
           it != oldSystemPackages.end() )
        {
          auto & [_, maybeLockedPackage] = *it;
          if ( maybeLockedPackage.has_value() )
            {
              if ( auto oldDescriptorPair = oldDescriptors.find( iid );
                   oldDescriptorPair != oldDescriptors.end() )
                {
                  auto & [_, oldDescriptor] = *oldDescriptorPair;
                  /* At this point we know the same _iid_ is both locked in the
                   * old lockfile and present in the new manifest.
                   *
                   * Don't use a locked input if the package has changed.
                   * The fields handled below control what the package actually
                   * *is* while:
                   * - `optional' and `systems' control how we behave if
                   *   resolution fails, but they don't change the package.
                   * - `priority' is a setting for `mkEnv' and is passed through
                   *   without effecting resolution.
                   * - `group' is handled below. */
                  if ( ( descriptor.name == oldDescriptor.name )
                       && ( descriptor.path == oldDescriptor.path )
                       && ( descriptor.version == oldDescriptor.version )
                       && ( descriptor.semver == oldDescriptor.semver )
                       && ( descriptor.subtree == oldDescriptor.subtree )
                       && ( descriptor.input == oldDescriptor.input ) )
                    {
                      if ( descriptor.group == oldDescriptor.group )
                        {
                          // TODO: check that input is still present in a
                          // registry somewhere?
                          return maybeLockedPackage->input;
                        }

                      /* The group has changed but the package hasn't, so we'll
                       * return this input below if we don't ever find a package
                       * with the correct group.
                       * If packages have come from multiple different wrong
                       * groups, just return the first one we encounter.
                       * We could come up with a better heuristic like most
                       * packages or newest, or we could try resolving in all
                       * of them.
                       * For now, don't get too fancy. */
                      if ( ! wrongGroupInput.has_value() )
                        {
                          wrongGroupInput = maybeLockedPackage->input;
                        }
                    }
                }
            }
        }
    }
  // TODO: check that input is still present in a registry somewhere?
  return wrongGroupInput;
}


/* -------------------------------------------------------------------------- */

ResolutionResult
Environment::tryResolveGroup( const InstallDescriptors & group,
                              const System &             system )
{
  /* List of resolution failures to group descriptors with the inputs they
   * failed to resolve in. */
  ResolutionFailure failure;

  /* If there is an existing lock and it has this group pinned to an existing
   * input+rev try to use it to resolve the group.
   * If we fail collect a list of failed descriptors; presumably these are
   * new group members. */
  std::optional<pkgdb::PkgDbInput> oldGroupInput;
  if ( auto oldLockfile = this->getOldLockfile(); oldLockfile.has_value() )
    {
      auto lockedInput
        = getGroupInput( group, *this->getOldLockfile(), system );
      if ( lockedInput.has_value() )
        {
          RegistryInput        registryInput( *lockedInput );
          nix::ref<nix::Store> store = this->getStore();
          oldGroupInput = pkgdb::PkgDbInput( store, registryInput );

          auto maybeResolved
            = this->tryResolveGroupIn( group, *oldGroupInput, system );

          /* If we're able to resolve the group with the same input+rev as the
           * old lockfile's pin, then we're done. */
          // XXX: I tried `std::variant( overloaded { ... } )' pattern here but
          //      template deduction failed with `gcc'.
          //      Rather than adding deduction guides and stuff `std::get_if'
          //      is fine here.
          if ( const SystemPackages * resolved
               = std::get_if<SystemPackages>( &maybeResolved ) )
            {
              return *resolved;
            }

          if ( const InstallID * iid
               = std::get_if<InstallID>( &maybeResolved ) )
            {
              failure.push_back( std::pair<InstallID, std::string> {
                *iid,
                oldGroupInput->getDbReadOnly()->lockedRef.string } );
            }
          else
            {
              throw ResolutionFailureException(
                "we thought this was an unreachable error" );
            }
        }
    }

  for ( const auto & [_, input] : *this->getPkgDbRegistry() )
    {
      /* If we already tried to resolve in this input - skip it. */
      if ( ! oldGroupInput.has_value() || ( ( *input ) == ( *oldGroupInput ) ) )
        {
          {
            auto maybeResolved
              = this->tryResolveGroupIn( group, *input, system );
            if ( const SystemPackages * resolved
                 = std::get_if<SystemPackages>( &maybeResolved ) )
              {
                return *resolved;
              }
            else if ( const InstallID * iid
                      = std::get_if<InstallID>( &maybeResolved ) )
              {
                failure.push_back( std::pair<InstallID, std::string> {
                  *iid,
                  input->getDbReadOnly()->lockedRef.string } );
              }
            else
              {
                throw ResolutionFailureException(
                  "we thought this was an unreachable error" );
              }
          }
        }
    }
  return failure;
}


/* -------------------------------------------------------------------------- */

std::variant<InstallID, SystemPackages>
Environment::tryResolveGroupIn( const InstallDescriptors & group,
                                const pkgdb::PkgDbInput &  input,
                                const System &             system )
{
  std::unordered_map<InstallID, std::optional<pkgdb::row_id>> pkgRows;

  for ( const auto & [iid, descriptor] : group )
    {
      /* Skip unrequested systems. */
      if ( descriptor.systems.has_value()
           && ( std::find( descriptor.systems->begin(),
                           descriptor.systems->end(),
                           system )
                == descriptor.systems->end() ) )
        {
          pkgRows.emplace( iid, std::nullopt );
          continue;
        }

      /* Try resolving. */
      std::optional<pkgdb::row_id> maybeRow
        = this->tryResolveDescriptorIn( descriptor, input, system );
      if ( maybeRow.has_value() || descriptor.optional )
        {
          pkgRows.emplace( iid, maybeRow );
        }
      else { return iid; }
    }

  /* Convert to `LockedPackageRaw's */
  SystemPackages pkgs;
  LockedInputRaw lockedInput( input );
  auto           dbRO = input.getDbReadOnly();
  for ( const auto & [iid, maybeRow] : pkgRows )
    {
      if ( maybeRow.has_value() )
        {
          pkgs.emplace( iid,
                        Environment::lockPackage( lockedInput,
                                                  *dbRO,
                                                  *maybeRow,
                                                  group.at( iid ).priority ) );
        }
      else { pkgs.emplace( iid, std::nullopt ); }
    }

  return pkgs;
}


/* -------------------------------------------------------------------------- */

void
Environment::lockSystem( const System & system )
{
  /* This should only be called from `Environment::createLock()' after
   * initializing `lockfileRaw'. */
  assert( this->lockfileRaw.has_value() );
  SystemPackages pkgs;

  auto groups = this->getUnlockedGroups( system );

  /* Try resolving unresolved groups. */
  std::vector<ResolutionFailure> failures;
  std::stringstream              msg;
  msg << "failed to resolve some package(s):" << std::endl;
  for ( auto group = groups.begin(); group != groups.end(); )
    {
      ResolutionResult maybeResolved = this->tryResolveGroup( *group, system );
      std::visit(
        overloaded {
          /* Add to pkgs if the group was successfully resolved. */
          [&]( SystemPackages & resolved )
          {
            pkgs.merge( resolved );
            group = groups.erase( group );
          },

          /* Otherwise add a description of the resolution failure to msg. */
          [&]( const ResolutionFailure & failure )
          {
            // TODO: Throw sooner rather than trying to resolve every group?
            /* We should only hit this on the first iteration. */
            if ( failure.empty() )
              {
                throw ResolutionFailureException(
                  "no inputs found to search for packages" );
              }
            /* Print group name if this isn't the default group. */
            if ( auto descriptor = group->begin();
                 descriptor != group->end()
                 && descriptor->second.group.has_value() )
              {

                msg << "  in group `" << *descriptor->second.group << "': ";
              }
            else { msg << "  in default group:"; }
            msg << std::endl;
            for ( const auto & [iid, url] : failure )
              {
                msg << "    failed to resolve `" << iid << "' in input `" << url
                    << '\'';
              }

            ++group;
          } },
        maybeResolved );
    }

  if ( ! groups.empty() ) { throw ResolutionFailureException( msg.str() ); }

  /* Copy over old lockfile entries we want to keep.
   * Make sure to update the priority if the entry was copied over from
   * the old. */
  if ( auto oldLockfile = this->getOldLockfile();
       oldLockfile.has_value()
       && oldLockfile->getLockfileRaw().packages.contains( system ) )
    {
      SystemPackages systemPackages
        = oldLockfile->getLockfileRaw().packages.at( system );
      auto oldDescriptors = oldLockfile->getDescriptors();
      for ( const auto & group : this->getLockedGroups( system ) )
        {
          for ( const auto & [iid, descriptor] : group )
            {
              if ( auto oldLockedPackagePair = systemPackages.find( iid );
                   oldLockedPackagePair != systemPackages.end() )
                {
                  pkgs.emplace( *oldLockedPackagePair );
                  pkgs.at( iid )->priority = descriptor.priority;
                }
            }
        }
    }

  this->lockfileRaw->packages.emplace( system, std::move( pkgs ) );
}


/* -------------------------------------------------------------------------- */

Lockfile
Environment::createLockfile()
{
  if ( ! this->lockfileRaw.has_value() )
    {
      this->lockfileRaw           = LockfileRaw {};
      this->lockfileRaw->manifest = this->getManifestRaw();
      this->lockfileRaw->registry = this->getCombinedRegistryRaw();
      for ( const auto & system : this->getSystems() )
        {
          this->lockSystem( system );
        }
    }
  Lockfile lockfile( *this->lockfileRaw );
  lockfile.removeUnusedInputs();
  return lockfile;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
