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

Groups
getGroupedDescriptors( const InstallDescriptors & descriptors )
{
  /* Group all packages into a map with group name as key. */
  Groups grouped;
  for ( const auto & [iid, desc] : descriptors )
    {
      // TODO: Use manifest options to decide how ungrouped descriptors
      //       are grouped.
      /* For now add all descriptors without a group to TOPLEVEL_GROUP_NAME.
       * Note that TOPLEVEL_GROUP_NAME is reserved but not forbidden; if a user
       * puts a package in the "toplevel" group, it will end up in the same
       * group as any packages without an explicit group. */
      if ( ! desc.group.has_value() )
        {
          grouped.try_emplace( GroupName( TOPLEVEL_GROUP_NAME ),
                               InstallDescriptors {} );
          grouped.at( GroupName( TOPLEVEL_GROUP_NAME ) ).emplace( iid, desc );
        }
      else
        {
          grouped.try_emplace( *desc.group, InstallDescriptors {} );
          grouped.at( *desc.group ).emplace( iid, desc );
        }
    }

  return grouped;
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
