/* ========================================================================== *
 *
 * @file resolver/manifest.cc
 *
 * @brief An abstract description of an environment in its unresolved state.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <filesystem>
#include <optional>
#include <string>
#include <type_traits>
#include <unordered_map>
#include <utility>
#include <vector>

#include "flox/core/types.hh"
#include "flox/core/util.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/registry.hh"
#include "flox/resolver/descriptor.hh"
#include "flox/resolver/manifest-raw.hh"
#include "flox/resolver/manifest.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

std::vector<InstallDescriptors>
getGroupedDescriptors( const InstallDescriptors & descriptors )
{
  /* Group all packages into a map with group name as key. */
  std::unordered_map<GroupName, InstallDescriptors> grouped;
  InstallDescriptors                                defaultGroup;
  for ( const auto & [iid, desc] : descriptors )
    {
      // TODO: Use manifest options to decide how ungrouped descriptors
      //       are grouped.
      /* For now add all descriptors without a group to `defaultGroup`. */
      if ( ! desc.group.has_value() ) { defaultGroup.emplace( iid, desc ); }
      else
        {
          grouped.try_emplace( *desc.group, InstallDescriptors {} );
          grouped.at( *desc.group ).emplace( iid, desc );
        }
    }

  /* Add all groups to a vector.
   * Don't use a map with group name because the defaultGroup doesn't have
   * a name. */
  std::vector<InstallDescriptors> allDescriptors;
  if ( ! defaultGroup.empty() ) { allDescriptors.emplace_back( defaultGroup ); }
  for ( const auto & [_, group] : grouped )
    {
      allDescriptors.emplace_back( group );
    }
  return allDescriptors;
}


/* -------------------------------------------------------------------------- */

/* Instantiate templates. */

template class ManifestBase<ManifestRaw>;
template class ManifestBase<ManifestRawGA>;
template class ManifestBase<GlobalManifestRaw>;
template class ManifestBase<GlobalManifestRawGA>;

template class EnvironmentManifestBase<ManifestRaw>;
template class EnvironmentManifestBase<ManifestRawGA>;


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
