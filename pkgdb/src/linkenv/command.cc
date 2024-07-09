/* ========================================================================== *
 *
 * @file linkenv/command.cc
 *
 * @brief Link a previously built environment.
 *
 *
 * -------------------------------------------------------------------------- */

#include "flox/linkenv/command.hh"

#include <nix/local-fs-store.hh>

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

namespace flox::linkenv {

/* -------------------------------------------------------------------------- */

LinkEnvCommand::LinkEnvCommand() : parser( "linkenv" )
{
  this->parser.add_description( "Link a previously built environment." );

  this->parser.add_argument( "--out-link", "-o" )
    .help( "path to link resulting environment or builder to" )
    .required()
    .metavar( "OUT-LINK" )
    .action( [&]( const std::string & str ) { this->outLink = str; } );

  this->parser.add_argument( "--store-path" )
    .help( "the store path to create the link to" )
    .required()
    .metavar( "STORE-PATH" )
    .action( [&]( const std::string & str ) { this->storePath = str; } );
}


/* -------------------------------------------------------------------------- */

int
LinkEnvCommand::run()
{
  auto store = this->getStore();
  auto state = this->getState();

  std::filesystem::path path( this->storePath.value() );
  if ( ! std::filesystem::exists( path ) )
    {
      std::cerr << "No such store-path: " << path << '\n';
      return EXIT_FAILURE;
    }

  nix::StorePath storePath( std::string( path.filename() ) );
  debugLog( "linking environment" );
  writeOutLink( store, storePath, this->outLink.value() );
  /* Print the resulting store path */
  nlohmann::json result
    = { { "store_path", store->printStorePath( storePath ) } };
  std::cout << result.dump() << '\n';
  return EXIT_SUCCESS;
}

/* -------------------------------------------------------------------------- */

}  // namespace flox::linkenv

/* -------------------------------------------------------------------------- */


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
