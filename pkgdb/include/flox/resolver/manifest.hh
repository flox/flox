/* ========================================================================== *
 *
 * @file flox/resolver/manifest.hh
 *
 * @brief An abstract description of an environment in its unresolved state.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <optional>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

#include <nix/config.hh>
#include <nix/globals.hh>
#include <nix/ref.hh>

#include "compat/concepts.hh"
#include "flox/core/nix-state.hh"
#include "flox/core/types.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/registry.hh"
#include "flox/resolver/manifest-raw.hh"


/* -------------------------------------------------------------------------- */

/* Forward Declarations. */

namespace flox::resolver {
struct ManifestDescriptor;
}  // namespace flox::resolver

namespace nix {
class Store;
}


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

/** @brief Read a flox::resolver::ManifestBase from a file. */
template<manifest_raw_type RawType>
static inline RawType
readManifestFromPath( const std::filesystem::path & manifestPath )
{
  if ( ! std::filesystem::exists( manifestPath ) )
    {
      throw InvalidManifestFileException( "no such path: "
                                          + manifestPath.string() );
    }
  return readAndCoerceJSON( manifestPath );
}


/* -------------------------------------------------------------------------- */

/**
 * @brief A _global_ manifest containing only `registry` and `options` fields.
 *
 * This is intended for use outside of any particular project to supply inputs
 * for `flox search`, `flox show`, and similar commands.
 *
 * In the context of a project this file may be referenced, but its contents
 * will always yield priority to the project's own manifest, and in cases where
 * settings or inputs are not declared in a project, they may be automatically
 * added from the global manifest.
 */
template<manifest_raw_type RawType>
class ManifestBase
{

protected:

  /* We need these `protected' so they can be set by `Manifest'. */
  // NOLINTBEGIN(cppcoreguidelines-non-private-member-variables-in-classes)
  // TODO: remove `manifestPath'
  RawType     manifestRaw;
  RegistryRaw registryRaw;
  // NOLINTEND(cppcoreguidelines-non-private-member-variables-in-classes)

  /** @brief Initialize @a registryRaw from @a manifestRaw. */
  template<manifest_raw_type _RawType = RawType>
  typename std::enable_if<std::derived_from<_RawType, GlobalManifestRaw>,
                          void>::type
  initRegistry()
  {
    if ( this->manifestRaw.registry.has_value() )
      {
        this->registryRaw = *this->manifestRaw.registry;
      }
  }

  /** @brief Initialize @a registryRaw from @a manifestRaw. */
  template<manifest_raw_type _RawType = RawType>
  typename std::enable_if<std::derived_from<_RawType, GlobalManifestRawGA>,
                          void>::type
  initRegistry()
  {
    this->registryRaw = getGARegistry();
  }


public:

  using rawType = RawType;

  virtual ~ManifestBase()                  = default;
  ManifestBase( const ManifestBase & )     = default;
  ManifestBase( ManifestBase && ) noexcept = default;

  ManifestBase()
  {
    this->manifestRaw.check();
    this->initRegistry();
  }

  explicit ManifestBase( RawType raw ) : manifestRaw( std::move( raw ) )
  {
    this->manifestRaw.check();
    this->initRegistry();
  }

  explicit ManifestBase( const std::filesystem::path & manifestPath )
    : manifestRaw( readManifestFromPath<RawType>( manifestPath ) )
  {
    this->manifestRaw.check();
    this->initRegistry();
  }

  ManifestBase &
  operator=( const ManifestBase & )
    = default;

  ManifestBase &
  operator=( ManifestBase && ) noexcept
    = default;

  [[nodiscard]] const RawType &
  getManifestRaw() const
  {
    return this->manifestRaw;
  }

  [[nodiscard]] const RegistryRaw &
  getRegistryRaw() const
  {
    return this->registryRaw;
  }

  /* Ignore linter warning about copying params because `nix::ref` is just
   * a pointer ( `std::shared_pointer' with a `nullptr` check ). */
  [[nodiscard]] RegistryRaw
  getLockedRegistry( const nix::ref<nix::Store> & store
                     = NixStoreMixin().getStore() ) const
  {
    return lockRegistry( this->getRegistryRaw(), store );
  }

  /** @brief Get the list of systems requested by the manifest. */
  [[nodiscard]] std::vector<System>
  getSystems() const
  {
    const auto & manifest = this->getManifestRaw();
    if ( manifest.options.has_value() && manifest.options->systems.has_value() )
      {
        return *manifest.options->systems;
      }
    return std::vector<System> { nix::settings.thisSystem.get() };
  }

  [[nodiscard]] pkgdb::PkgQueryArgs
  getBaseQueryArgs() const
  {
    pkgdb::PkgQueryArgs args;
    if ( ! this->manifestRaw.options.has_value() ) { return args; }

    if ( this->manifestRaw.options->systems.has_value() )
      {
        args.systems = *this->manifestRaw.options->systems;
      }

    if ( this->manifestRaw.options->allow.has_value() )
      {
        if ( this->manifestRaw.options->allow->unfree.has_value() )
          {
            args.allowUnfree = *this->manifestRaw.options->allow->unfree;
          }
        if ( this->manifestRaw.options->allow->broken.has_value() )
          {
            args.allowBroken = *this->manifestRaw.options->allow->broken;
          }
        args.licenses = this->manifestRaw.options->allow->licenses;
      }

    if ( this->manifestRaw.options->semver.has_value()
         && this->manifestRaw.options->semver->preferPreReleases.has_value() )
      {
        args.preferPreReleases
          = *this->manifestRaw.options->semver->preferPreReleases;
      }
    return args;
  }


}; /* End class `ManifestBase' */


/* -------------------------------------------------------------------------- */

template<manifest_raw_type RawType>
class GlobalManifestBase : public ManifestBase<RawType>
{

public:

  ~GlobalManifestBase() override                   = default;
  GlobalManifestBase()                             = default;
  GlobalManifestBase( const GlobalManifestBase & ) = default;
  GlobalManifestBase( GlobalManifestBase && )      = default;

  GlobalManifestBase &
  operator=( const GlobalManifestBase & )
    = default;

  GlobalManifestBase &
  operator=( GlobalManifestBase && )
    = default;

  explicit GlobalManifestBase( RawType raw )
    : ManifestBase<RawType>( std::move( raw ) )
  {}

  explicit GlobalManifestBase( const std::filesystem::path & manifestPath )
    : ManifestBase<RawType>( manifestPath )
  {}


}; /* End class `GlobalManifestBase' */


/* -------------------------------------------------------------------------- */

using GlobalManifest   = GlobalManifestBase<GlobalManifestRaw>;
using GlobalManifestGA = GlobalManifestBase<GlobalManifestRawGA>;


/* -------------------------------------------------------------------------- */

/** @brief A map of _install IDs_ to _manifest descriptors_. */
using InstallDescriptors = std::unordered_map<InstallID, ManifestDescriptor>;

/**
 * @brief Returns all descriptors, grouping those with a _group_ field, and
 *        returning those without a group field as a map with a
 *        single element.
 */
[[nodiscard]] std::vector<InstallDescriptors>
getGroupedDescriptors( const InstallDescriptors & descriptors );


/* -------------------------------------------------------------------------- */


/** @brief Description of an environment in its _unlocked_ form. */
template<manifest_raw_type RawType>
class EnvironmentManifestBase : public ManifestBase<RawType>
{

private:

  /**
   * A map of _install ID_ to _descriptors_, being descriptions/requirements
   * of a dependency.
   */
  InstallDescriptors descriptors;


  /**
   * @brief Assert the validity of the manifest, throwing an exception if it
   *        contains invalid fields.
   *
   * This checks that:
   * - The raw manifest is valid.
   * - If `install.<IID>.systems` is set, then `options.systems` is also set.
   * - All `install.<IID>.systems` are in `options.systems`.
   */
  void
  check() const
  {
    const auto & raw = this->getManifestRaw();
    raw.check();
    std::optional<std::vector<std::string>> maybeSystems;
    if ( auto maybeOpts = raw.options; maybeOpts.has_value() )
      {
        maybeSystems = maybeOpts->systems;
      }

    for ( const auto & [iid, desc] : this->descriptors )
      {
        if ( ! desc.systems.has_value() ) { continue; }
        if ( ! maybeSystems.has_value() )
          {
            throw InvalidManifestFileException(
              "descriptor `install." + iid
              + "' specifies `systems' but no `options.systems' are specified"
                " in the manifest." );
          }
        for ( const auto & system : *desc.systems )
          {
            if ( std::find( maybeSystems->begin(), maybeSystems->end(), system )
                 == maybeSystems->end() )
              {
                std::stringstream msg;
                msg << "descriptor `install." << iid << "' specifies system `"
                    << system
                    << "' which is not in `options.systems' in the manifest.";
                throw InvalidManifestFileException( msg.str() );
              }
          }
      }
  }

  /** @brief Initialize @a descriptors from @a manifestRaw. */
  void
  initDescriptors()
  {
    if ( ! this->manifestRaw.install.has_value() ) { return; }
    for ( const auto & [iid, raw] : *this->manifestRaw.install )
      {
        /* An empty/null descriptor uses `name' of the attribute. */
        if ( raw.has_value() )
          {
            this->descriptors.emplace( iid, ManifestDescriptor( iid, *raw ) );
          }
        else
          {
            ManifestDescriptor manDesc;
            manDesc.name = iid;
            this->descriptors.emplace( iid, std::move( manDesc ) );
          }
      }
    this->check();
  }


public:

  ~EnvironmentManifestBase() override                        = default;
  EnvironmentManifestBase()                                  = default;
  EnvironmentManifestBase( const EnvironmentManifestBase & ) = default;
  EnvironmentManifestBase( EnvironmentManifestBase && )      = default;

  explicit EnvironmentManifestBase( RawType raw )
    : ManifestBase<RawType>( std::move( raw ) )
  {
    this->initDescriptors();
  }

  explicit EnvironmentManifestBase( const std::filesystem::path & manifestPath )
    : ManifestBase<RawType>( readManifestFromPath<RawType>( manifestPath ) )
  {
    this->initDescriptors();
  }

  EnvironmentManifestBase &
  operator=( const EnvironmentManifestBase & )
    = default;

  EnvironmentManifestBase &
  operator=( EnvironmentManifestBase && )
    = default;

  /** @brief Get _descriptors_ from the manifest's `install' field. */
  [[nodiscard]] const InstallDescriptors &
  getDescriptors() const
  {
    return this->descriptors;
  }

  /**
   * @brief Returns all descriptors, grouping those with a _group_ field, and
   *        returning those without a group field as a map with a
   *        single element.
   */
  [[nodiscard]] std::vector<InstallDescriptors>
  getGroupedDescriptors() const
  {
    return flox::resolver::getGroupedDescriptors( this->descriptors );
  }


}; /* End class `EnvironmentManifestBase' */


/* -------------------------------------------------------------------------- */

using EnvironmentManifest   = EnvironmentManifestBase<ManifestRaw>;
using EnvironmentManifestGA = EnvironmentManifestBase<ManifestRawGA>;


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
