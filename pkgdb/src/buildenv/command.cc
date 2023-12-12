/* ========================================================================== *
 *
 * @file buildenv/command.cc
 *
 * @brief Evaluate and build a locked environment.
 *
 *
 * -------------------------------------------------------------------------- */

#include "flox/buildenv/command.hh"

/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

BuildEnvCommand::BuildEnvCommand()
{
  this->parser.add_description( "Evaluate and build a locked environment" );
  this->parser.add_argument( "lockfile" )
    .help( "inline JSON or path to lockfile" )
    .required()
    .metavar( "LOCKFILE" )
    .action( [&]( const std::string & str )
             { this->lockfileContent = readOrCoerceJSON( str ); } );

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

void
BuildEnvCommand::run( ref<nix::Store> store ) override
{

  if ( nix::lvlDebug <= nix::verbosity )
    {
      logger->log( nix::Verbosity::lvlDebug,
                   "lockfile: " + this->lockfileContent );
    }

  LockfileRaw lockfileRaw = nlohmann::json::parse( lockfileContent );
  auto        lockfile    = Lockfile( lockfileRaw );
  auto        state       = getEvalState();

  auto system    = this->system.value_or( nix::settings.thisSystem.get() );
  auto storePath = flox::createFloxEnv( *state, lockfile, system );

  /* Print the resulting store path */
  std::cout << store->printStorePath( storePath ) << std::endl;

  auto localStore = store.dynamic_pointer_cast<LocalFSStore>();

  // TODO: Make a read error
  if ( localStore == nullptr )
    {
      throw FloxException( "store is not a LocalFSStore" );
    }

  if ( outLink.has_value() )
    {
      auto outLinkPath
        = localStore->addPermRoot( storePath, absPath( outLink.value() ) );
      if ( nix::lvlDebug <= nix::verbosity )
        {
          logger->log( nix::Verbosity::lvlDebug, "outLinkPath: " + outLinkPath )
        }
    }
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
