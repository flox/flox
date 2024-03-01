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

static void
writeOutLink( const nix::ref<nix::Store> & store,
              const nix::StorePath &       storePath,
              const nix::Path &            path )
{
  auto localStore = store.dynamic_pointer_cast<nix::LocalFSStore>();
  if ( localStore == nullptr )
    {
      throw flox::FloxException( "store is not a LocalFSStore" );
    }

  auto outLinkPath = localStore->addPermRoot( storePath, nix::absPath( path ) );

  if ( nix::lvlDebug <= nix::verbosity )
    {
      nix::logger->log( nix::Verbosity::lvlDebug,
                        "outLinkPath: " + outLinkPath );
    }
}

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

  this->parser.add_argument( "--out-link", "-o" )
    .help( "path to link resulting environment or builder to" )
    .metavar( "OUT-LINK" )
    .action( [&]( const std::string & str ) { this->outLink = str; } );

  this->parser.add_argument( "--store-path" )
    .help( "the store path to create the link to" )
    .metavar( "STORE-PATH" )
    .action( [&]( const std::string & str ) { this->storePath = str; } );

  this->parser.add_argument( "--system", "-s" )
    .help( "system to build for" )
    .metavar( "SYSTEM" )
    .nargs( 1 )
    .action( [&]( const std::string & str ) { this->system = str; } );

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

  resolver::LockfileRaw lockfileRaw = this->lockfileContent;
  auto lockfile = resolver::Lockfile( std::move( lockfileRaw ) );
  auto system   = this->system.value_or( nix::settings.thisSystem.get() );

  auto store = this->getStore();
  auto state = this->getState();

  if ( this->storePath.has_value() && this->outLink.has_value() )
    {
      std::filesystem::path path( this->storePath.value() );
      nix::StorePath        storePath( std::string( path.filename() ) );
      debugLog( nix::fmt(
        "store path was provided, skipping build: store_path=%s, out_link=%s",
        store->printStorePath( storePath ),
        this->outLink.value() ) );
      writeOutLink( store, storePath, this->outLink.value() );
      /* Print the resulting store path */
      nlohmann::json result
        = { { "store_path", store->printStorePath( storePath ) } };
      std::cout << result.dump() << '\n';
      return EXIT_SUCCESS;
    }
  else if ( this->storePath.has_value() && ! this->outLink.has_value() )
    {
      throw BuildenvInvalidArguments(
        "'--store-path' requires the '--out-link' flag" );
    }

  debugLog( "building environment" );

  auto storePath = createFloxEnv( state, lockfile, system );

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

  if ( outLink.has_value() )
    {
      debugLog( "writing out-link" );
      writeOutLink( store, storePath, outLink.value() );
    }

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
