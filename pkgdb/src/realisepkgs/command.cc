/* ========================================================================== *
 *
 * @file realisepkgs/command.cc
 *
 * @brief Evaluate and build a locked environment.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nix/local-fs-store.hh>

#include "flox/realisepkgs/command.hh"
#include "flox/realisepkgs/realise.hh"
#include "flox/resolver/lockfile.hh"

/* -------------------------------------------------------------------------- */

namespace flox::realisepkgs {

/* -------------------------------------------------------------------------- */

RealisePkgsCommand::RealisePkgsCommand() : parser( "realisepkgs" )
{
  this->parser.add_description( "Realise packages from a locked environment" );
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
}


/* -------------------------------------------------------------------------- */

int
RealisePkgsCommand::run()
{

  debugLog( "lockfile: " + this->lockfileContent.dump( 2 ) );

  auto system = this->system.value_or( nix::settings.thisSystem.get() );

  auto store = this->getStore();
  auto state = this->getState();

  debugLog( "realising packages" );

  auto pkgs = realiseFloxEnvPackages( state, this->lockfileContent, system );

  /* Print the store paths rendered a la `nix build --print-out-paths` */
  for ( const auto & pkg : pkgs )
    {
      if ( pkg.active ) {
        std::cout << pkg.path << '\n';
      }
    }

  return EXIT_SUCCESS;
}

/* -------------------------------------------------------------------------- */

}  // namespace flox::realisepkgs

/* -------------------------------------------------------------------------- */


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
