#include <nix/builtins/buildenv.hh>
#include <nix/command.hh>
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

      auto vFlake = state.allocValue();
      flake::callFlake( state, package_flake, *vFlake );
      state.forceAttrs( *vFlake, noPos, "while parsing flake" );


      auto output = vFlake->attrs->get( state.symbols.create( "outputs" ) );


      /* evaluate the package */
      for ( auto path_segment : package.attrPath )
        {
          state.forceAttrs( *output->value,
                            output->pos,
                            "while parsing cached flake data" );

          auto found
            = output->value->attrs->get( state.symbols.create( path_segment ) );
          if ( ! found )
            {
              std::ostringstream str;
              output->value->print( state.symbols, str );
              throw Error( "Attribute '%s' not found in set '%s'",
                           path_segment,
                           str.str() );
            }
          output = found;
        }


      auto package_drv = getDerivation( state, *output->value, false );

      for ( auto output : package_drv->queryOutputs() )
        {
          if ( ! output.second.has_value() )
            {
              continue;
            }  // skip outputs without path
          pkgs.emplace_back(
            state.store->printStorePath( output.second.value() ),
            true,
            package.priority );
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


struct CmdBuildEnv : nix::EvalCommand
{
  std::string lockfile_content;

  CmdBuildEnv()
  {
    // expectArgs( { .label = "lockfile", .handler = { &lockfile_content } } );

    addFlag( { .longName    = "lockfile",
               .shortName   = 'l',
               .description = "locked manifest",
               .labels      = { "lockfile" },
               .handler     = { &lockfile_content } } );
  }

  std::string
  description() override
  {
    return "build flox env";
  }

  std::string
  doc() override
  {
    return "TODO";
  }


  void
  run( ref<Store> store ) override
  {
    assert( parent );
    nix::MultiCommand * toplevel = parent;
    while ( toplevel->parent ) { toplevel = toplevel->parent; }

    printf( "lockfile: %s\n", lockfile_content.c_str() );

    LockfileRaw lockfile_raw = nlohmann::json::parse( lockfile_content );

    auto state = getEvalState();

    auto system   = std::string( "aarch64-darwin" );
    auto lockfile = Lockfile( lockfile_raw );


    auto store_path = flox::createUserEnv( *state, lockfile, system, false );

    printf( "store_path: %s\n", store->printStorePath( store_path ).c_str() );

    // showHelp( self, getFloxArgs( *this ) );
  }
};

static auto rCmdBuildEnv = nix::registerCommand<CmdBuildEnv>( "build-env" );


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
