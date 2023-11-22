#include <nix/derivations.hh>
#include <nix/eval-inline.hh>
#include <nix/eval.hh>
#include <nix/flake/flake.hh>
#include <nix/globals.hh>
#include <nix/local-fs-store.hh>
#include <nix/path-with-outputs.hh>
#include <nix/profiles.hh>
#include <nix/shared.hh>
#include <nix/store-api.hh>
#include <nix/user-env.hh>
#include <nix/util.hh>
#include <nlohmann/json.hpp>

#include <flox/resolver/lockfile.hh>


/* -------------------------------------------------------------------------- */

namespace flox {
using namespace nix;
using namespace flox::resolver;

/* -------------------------------------------------------------------------- */

bool
createUserEnv( EvalState &          state,
               resolver::Lockfile & lockfile,
               System &             system,
               bool                 keepDerivations )
{


  auto packages = lockfile.getLockfileRaw().packages.find( system );
  if ( packages == lockfile.getLockfileRaw().packages.end() )
    {
      throw Error( "No packages found for system '%s'", system );
    }

  /* extract all packages */

  std::vector<resolver::LockedPackageRaw> locked_packages;

  for ( auto const & package : packages->second )
    {
      if ( ! package.second.has_value() ) { continue; }
      auto const & locked_package = package.second.value();
      locked_packages.push_back( locked_package );
    }

  /**
   * extract derivations
   */

  for ( auto const & package : locked_packages )
    {

      auto package_input_ref = FlakeRef( package.input );
      auto package_flake
        = flake::lockFlake( state, package_input_ref, flake::LockFlags {} );

      Value attr;
      flake::callFlake( state, package_flake, attr );
      /* evaluate the package */
      for ( auto path_segment : package.attrPath )
        {
          auto found = attr.attrs->find( state.symbols.create( path_segment ) );
          if ( found == attr.attrs->end() )
            {
              throw Error( "Attribute '%s' not found in flake '%s'",
                           path_segment,
                           package.input );
            }
          attr = *found->value;
        }

      auto package_drv = getDerivation( state, attr, false );
      if ( ! package_drv.has_value() )
        {
          throw Error( "Failed to get derivation for package '%s'",
                       package.input );
        }

    }
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
