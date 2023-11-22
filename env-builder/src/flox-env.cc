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
  StorePathSet references;
  std::vector<StorePathWithOutputs> drvsToBuild;
  Value                             environment_drvs;
  state.mkList( environment_drvs, locked_packages.size() );
  size_t n = 0;

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

      for ( const auto & [m, j] : enumerate( package_drv->queryOutputs() ) )
        {
          references.insert( *j.second );
        }


      if ( auto drvPath = package_drv->queryDrvPath() )
        {
          drvsToBuild.push_back( { *drvPath } );
          references.insert( *drvPath );
        }
      ( environment_drvs.listElems()[n++] = state.allocValue() )->mkAttrs( attr.attrs );


      // auto profile_d_scripts;
      // auto activateScript
    }

  state.store->buildPaths( toDerivedPaths( drvsToBuild ),
                           state.repair ? bmRepair : bmNormal );


  auto manifestFile
    = state.store->addTextToStore( "env-manifest.nix", nlohmann::json(lockfile.getLockfileRaw()).dump(), references);

  /* Get the environment builder expression. */
  Value envBuilder;
  state.eval( state.parseExprFromString(
#include "buildenv.nix.gen.hh"
                ,
                state.rootPath( CanonPath::root ) ),
              envBuilder );

  /* Construct a Nix expression that calls the user environment
   * builder with the manifest as argument. */
  auto attrs = state.buildBindings( 3 );
  state.mkStorePathString( manifestFile, attrs.alloc( "manifest" ) );
  attrs.insert( state.symbols.create( "derivations" ), &environment_drvs );
  Value args;
  args.mkAttrs( attrs );

  Value topLevel;
  topLevel.mkApp( &envBuilder, &args );

  debug( "evaluating user environment builder" );
  state.forceValue( topLevel,
                    [&]() { return topLevel.determinePos( noPos ); } );
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
