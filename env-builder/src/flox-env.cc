#include <nix/builtins/buildenv.hh>
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

StorePath
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
  StorePathSet                      references;
  std::vector<StorePathWithOutputs> drvsToBuild;
  Packages                          pkgs;
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

      for ( auto output : package_drv->queryOutputs() )
        {
          if ( ! output.second.has_value() ) { continue; } // skip outputs without path
          pkgs.emplace_back( output.second, true, package.priority );
          references.insert( output.second.value() );
        }

      if ( auto drvPath = package_drv->queryDrvPath() )
        {
          drvsToBuild.push_back( { *drvPath } );
        }


      // todo: auto profile_d_scripts;
      // todo: auto activateScript
    }

  // todo check if this builds `outputsToInstall` only
  state.store->buildPaths( toDerivedPaths( drvsToBuild ),
                           state.repair ? bmRepair : bmNormal );

  auto tempDir = createTempDir();
  buildProfile( tempDir, std::move( pkgs ) );

  /* Add the symlink tree to the store. */
  StringSink sink;
  dumpPath( tempDir, sink );

  auto narHash = hashString( htSHA256, sink.s );
  ValidPathInfo info {
            *state.store,
            "profile",
            FixedOutputInfo {
                .method = FileIngestionMethod::Recursive,
                .hash = narHash,
                .references = {
                    .others = std::move(references),
                    // profiles never refer to themselves
                    .self = false,
                },
            },
            narHash,
        };
  info.narSize = sink.s.size();

  StringSource source( sink.s );
  state.store->addToStore( info, source );

  return std::move( info.path );
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
