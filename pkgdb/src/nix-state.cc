/* ========================================================================== *
 *
 * @file flox/nix-state.cc
 *
 * @brief Manages a `nix` runtime state blob with associated helpers.
 *
 *
 * -------------------------------------------------------------------------- */

#include <cstddef>

#include <nix/config.hh>
#include <nix/error.hh>
#include <nix/eval.hh>
#include <nix/globals.hh>
#include <nix/logging.hh>
#include <nix/shared.hh>
#include <nix/util.hh>

#include "flox/core/nix-state.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

void
initNix()
{
  static bool didNixInit = false;
  if ( didNixInit ) { return; }

  // NOLINTNEXTLINE
  nix::setStackSize( ( std::size_t( 64 ) * 1024 ) * 1024 );
  nix::initNix();
  nix::initGC();
  /* Suppress benign warnings about `nix.conf'. */
  nix::Verbosity oldVerbosity = nix::verbosity;
  nix::verbosity              = nix::lvlError;
  nix::initPlugins();
  /* Restore verbosity to `nix' global setting */
  nix::verbosity = oldVerbosity;

  nix::evalSettings.enableImportFromDerivation.setDefault( false );
  nix::evalSettings.pureEval.setDefault( true );
  nix::evalSettings.useEvalCache.assign( true );
  nix::experimentalFeatureSettings.experimentalFeatures.assign(
    std::set( { nix::Xp::Flakes } ) );

  /* Use custom logger */
  bool printBuildLogs = nix::logger->isVerbose();
  if ( nix::logger != nullptr ) { delete nix::logger; }
  nix::logger = makeFilteredLogger( printBuildLogs );

  didNixInit = true;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
