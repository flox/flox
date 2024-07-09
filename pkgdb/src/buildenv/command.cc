/* ========================================================================== *
 *
 * @file buildenv/command.cc
 *
 * @brief Evaluate and build a locked environment.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nix/local-fs-store.hh>

#include "flox/buildenv/command.hh"
#include "flox/buildenv/realise.hh"
#include "flox/resolver/lockfile.hh"

/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

BuildEnvCommand::BuildEnvCommand() : parser( "buildenv" )
{
  this->parser.add_description( "Evaluate and build a locked environment, "
                                "optionally produce a container build script" );
  this->parser.add_argument( "lockfile" )
    .help( "inline JSON or path to lockfile" )
    .required()
    .metavar( "LOCKFILE" )
    .action( [&]( const std::string & str )
             { this->lockfileContent = parseOrReadJSONObject( str ); } );

  this->parser.add_argument( "--system", "-s" )
    .help( "system to build for" )
    .metavar( "SYSTEM" )
    .nargs( 1 )
    .action( [&]( const std::string & str ) { this->system = str; } );

  this->parser.add_argument( "--service-config" )
    .help( "path to service configuration file" )
    .metavar( "SERVICE-CONFIG" )
    .action( [&]( const std::string & str )
             { this->serviceConfigPath = str; } );

  this->parser.add_argument( "--container", "-c" )
    .help( "build a container builder script" )
    .nargs( 0 )
    .action( [&]( const auto & ) { this->buildContainer = true; } );
}


/* -------------------------------------------------------------------------- */

int
BuildEnvCommand::run()
{

  debugLog( "lockfile: " + this->lockfileContent.dump( 2 ) );

  auto system = this->system.value_or( nix::settings.thisSystem.get() );

  auto store = this->getStore();
  auto state = this->getState();

  debugLog( "building environment" );

  auto storePath = createFloxEnv( state,
                                  this->lockfileContent,
                                  this->serviceConfigPath,
                                  system );

  debugLog( "built environment: " + store->printStorePath( storePath ) );

  if ( buildContainer )
    {
      debugLog( "container requested, building container build script" );

      auto containerBuilderStorePath
        = createContainerBuilder( *state, storePath, system );

      debugLog( "built container builder: "
                + store->printStorePath( containerBuilderStorePath ) );

      storePath = containerBuilderStorePath;
    };

  /* Print the resulting store path */
  nlohmann::json result
    = { { "store_path", store->printStorePath( storePath ) } };
  std::cout << result.dump() << '\n';

  return EXIT_SUCCESS;
}

/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv

/* -------------------------------------------------------------------------- */


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
