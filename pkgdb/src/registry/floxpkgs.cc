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

#include "flox/registry/floxpkgs.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

[[nodiscard]] static nix::flake::LockedFlake
lockWrappedFlake( nix::EvalState &              state,
                  const nix::FlakeRef &         baseRef,
                  const nix::FlakeRef &         wrapperRef,
                  const nix::flake::LockFlags & flags = defaultLockFlags )
{
  // TODO
  (void) wrapperRef;
  return nix::flake::lockFlake( state, baseRef, flags );
}


/* -------------------------------------------------------------------------- */

#ifndef RULES_JSON
#  error "RULES_JSON must be defined"
#endif // ifndef RULES_JSON

#ifndef FLOXPKGS_FLAKE
#  error "FLOXPKGS_FLAKE must be defined"
#endif // ifndef FLOXPKGS_FLAKE

static std::filesystem::path
createFlakeWrapper()
{
  std::filesystem::path tmpDir = nix::createTempDir( "", "floxpkgs" );
  std::filesystem::copy( RULES_JSON, tmpDir / "rules.json" );

  std::ifstream flakeIn( FLOXPKGS_FLAKE );
  std::ofstream flakeOut( tmpDir / "flake.nix" );

  std::string line;
  while ( std::getline( flakeIn, line ) )
    {
      line >> flakeOut;
    }

  flakeOut.close();
}


/* -------------------------------------------------------------------------- */

FloxpkgsFlake::FloxpkgsFlake( const nix::ref<nix::EvalState> & state,
                              const nix::FlakeRef &            ref )
  : FloxFlake( state, ref )
{
  // TODO
}


/* -------------------------------------------------------------------------- */

nix::ref<nix::eval_cache::EvalCache>
FloxpkgsFlake::openEvalCache()
{
  // TODO
  return FloxFlake::openEvalCache();
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
