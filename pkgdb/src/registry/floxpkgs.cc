/* ========================================================================== *
 *
 * @file registry/floxpkgs.cc
 *
 * @brief Provides a specialized `FloxFlake' which applies rules/pre-processing
 *        to a `flake' before it is evaluated.
 *        This is used to implement the `floxpkgs' catalog.
 *
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <iostream>
#include <optional>

#include "flox/core/util.hh"
#include "flox/flox-flake.hh"
#include "flox/registry/floxpkgs.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

#ifndef RULES_JSON
#  error "RULES_JSON must be defined"
#endif  // ifndef RULES_JSON


/* -------------------------------------------------------------------------- */

[[nodiscard]] static const std::filesystem::path
getRulesFile()
{
  return nix::getEnv( "_PKGDB_NIXPKGS_RULES_JSON" ).value_or( RULES_JSON );
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Create a temporary directory containing a `flake.nix` which wraps
 *        @a nixpkgsRef, applying _rules_ from `rules.json`.
 */
std::filesystem::path
createWrappedFlakeDir( const nix::FlakeRef & nixpkgsRef )
{
  /* Create a temporary directory to put the filled out template and rules file
   * in. */
  std::filesystem::path tmpDir = nix::createTempDir();
  debugLog( "created temp dir for flake template: path=" + tmpDir.string() );
  std::filesystem::path rulesFilePath = getRulesFile();
  std::filesystem::copy( rulesFilePath, tmpDir / "rules.json" );

  /* Fill out the template with the flake references and the rules file path. */
  std::ofstream            flakeOut( tmpDir / "flake.nix" );
  static const std::string flakeTemplate =
#include "./floxpkgs/flake.nix.in.hh"
    ;
  std::istringstream flakeIn( flakeTemplate );
  std::string        line;
  while ( std::getline( flakeIn, line ) )
    {
      /* Inject URL */
      if ( line.find( "@NIXPKGS_URL@" ) != std::string::npos )
        {
          line.replace( line.find( "@NIXPKGS_URL@" ),
                        std::string( "@NIXPKGS_URL@" ).length(),
                        nixpkgsRef.to_string() );
        }

      /* Inject rules */
      if ( line.find( "@PKGDB_RULES_FILE@" ) != std::string::npos )
        {
          line.replace( line.find( "@PKGDB_RULES_FILE@" ),
                        std::string( "@PKGDB_RULES_FILE@" ).length(),
                        ( tmpDir / "rules.json" ).string() );
        }
      flakeOut << line << '\n';
    }
  flakeOut.close();
  debugLog( "filled out flake template: flake_ref=" + nixpkgsRef.to_string()
            + " rules_file_path=" + rulesFilePath.string() );

  /* Lock the filled out template to avoid spurious re-locking and silence the
   * "Added input ..." message. */
  flox::NixState           nixState;
  nix::ref<nix::EvalState> state = nixState.getState();
  nix::flake::LockFlags    flags;
  std::string              wrappedUrl = "path:" + tmpDir.string();
  nix::FlakeRef            wrappedRef = nix::parseFlakeRef( wrappedUrl );
  nix::flake::lockFlake( *state, wrappedRef, flags );
  debugLog( "locked flake template" );

  return tmpDir;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
