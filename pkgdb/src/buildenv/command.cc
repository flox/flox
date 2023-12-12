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
  this->parser.add_description( "Evaluate and build a locked environment" );
  this->parser.add_argument( "lockfile" )
    .help( "inline JSON or path to lockfile" )
    .required()
    .metavar( "LOCKFILE" )
    .action( [&]( const std::string & str )
             { this->lockfileContent = parseOrReadJSONObject( str ); } );

  this->parser.add_argument( "--out-link", "-o" )
    .help( "path to link resulting environment" )
    .metavar( "OUT-LINK" )
    .action( [&]( const std::string & str ) { this->outLink = str; } );

  this->parser.add_argument( "--system", "-s" )
    .help( "system to build for" )
    .metavar( "SYSTEM" )
    .nargs( 1 )
    .action( [&]( const std::string & str ) { this->system = str; } );
}


/* -------------------------------------------------------------------------- */

int
BuildEnvCommand::run()
{

  if ( nix::lvlDebug <= nix::verbosity )
    {
      nix::logger->log( nix::Verbosity::lvlDebug,
                        "lockfile: " + this->lockfileContent.dump( 2 ) );
    }

  resolver::LockfileRaw lockfileRaw = this->lockfileContent;
  auto                  lockfile    = resolver::Lockfile( std::move( lockfileRaw ) );
  auto                  store       = this->getStore();
  auto                  state       = this->getState();

  auto system    = this->system.value_or( nix::settings.thisSystem.get() );
  auto storePath = createFloxEnv( *state, lockfile, system );

  /* Print the resulting store path */
  std::cout << store->printStorePath( storePath ) << std::endl;

  auto localStore = store.dynamic_pointer_cast<nix::LocalFSStore>();

  // TODO: Make a read error
  if ( localStore == nullptr )
    {
      throw FloxException( "store is not a LocalFSStore" );
      return EXIT_FAILURE;
    }

  if ( outLink.has_value() )
    {
      auto outLinkPath
        = localStore->addPermRoot( storePath, nix::absPath( outLink.value() ) );
      if ( nix::lvlDebug <= nix::verbosity )
        {
          nix::logger->log( nix::Verbosity::lvlDebug,
                            "outLinkPath: " + outLinkPath );
        }
    }

  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
