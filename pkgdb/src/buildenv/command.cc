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

CmdBuildEnv::CmdBuildEnv()
{
  /* TODO
  addFlag( { .longName    = "lockfile",
             .shortName   = 'l',
             .description = "locked manifest",
             .labels      = { "lockfile" },
             .handler     = { &lockfile_content } } );

  addFlag( { .longName    = "out-link",
             .shortName   = 'o',
             .description = "output link",
             .labels      = { "out-link" },
             .handler     = { &out_link } } );

  addFlag( { .longName    = "system",
             .shortName   = 's',
             .description = "system",
             .labels      = { "system" },
             .handler     = { &system } } );
  */
}


/* -------------------------------------------------------------------------- */

void
CmdBuildEnv::run( ref<nix::Store> store ) override
{

  if ( nix::lvlDebug <= nix::verbosity )
    {
      logger->log( nix::Verbosity::lvlDebug,
                  "lockfile: " + this->lockfileContent );
    }

  LockfileRaw lockfileRaw = nlohmann::json::parse( lockfileContent );
  auto        lockfile     = Lockfile( lockfileRaw );
  auto        state        = getEvalState();

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
            logger->log( nix::Verbosity::lvlDebug,
                         "outLinkPath: " + outLinkPath )
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
