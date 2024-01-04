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
[[nodiscard]] static std::filesystem::path
createWrappedFlakeDir( const nix::FlakeRef & nixpkgsRef )
{
  std::filesystem::path tmpDir = nix::createTempDir();
  std::filesystem::copy( getRulesFile(), tmpDir / "rules.json" );

  std::ofstream            flakeOut( tmpDir / "flake.nix" );
  static const std::string flakeTemplate =
#include "./floxpkgs/flake.nix.in.hh"
    ;
  std::istringstream flakeIn( flakeTemplate );

  std::string line;
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

  debugLog( "Created wrapped flake in: `" + tmpDir.string() + '\'' );

  return tmpDir;
}


/* -------------------------------------------------------------------------- */

/** @brief Create a wrapped flake and return its _flake-ref_. */
[[nodiscard]] static nix::FlakeRef
createWrappedFlake( const nix::FlakeRef & nixpkgsRef )
{
  std::filesystem::path tmpDir = createWrappedFlakeDir( nixpkgsRef );
  return nix::parseFlakeRef( tmpDir.string() );
}


/* -------------------------------------------------------------------------- */

/** @brief Create and lock a wrapped flake. */
[[nodiscard]] static nix::flake::LockedFlake
createWrappedLockedFlake( nix::EvalState &      state,
                          const nix::FlakeRef & nixpkgsRef )
{
  nix::FlakeRef         ref   = createWrappedFlake( nixpkgsRef );
  nix::flake::LockFlags flags = defaultLockFlags;
  flags.updateLockFile        = true;
  flags.writeLockFile         = true;
  return flox::lockFlake( state, ref, flags );
}


/* -------------------------------------------------------------------------- */

FloxpkgsFlake::FloxpkgsFlake( const nix::ref<nix::EvalState> & state,
                              const nix::FlakeRef &            ref )
  : FloxFlake( state, createWrappedLockedFlake( *state, ref ) )
  , nixpkgsRef( ref )
{}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
