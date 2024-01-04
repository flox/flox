/* ========================================================================== *
 *
 * @file flox/registry/floxpkgs.hh
 *
 * @brief Provides a specialized `FloxFlake' which applies rules/pre-processing
 *        to a `flake' before it is evaluated.
 *        This is used to implement the `floxpkgs' catalog.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include "flox/flox-flake.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/**
 * @brief Provides a specialized `FloxFlake' which applies rules/pre-processing
 *        to a `flake' before it is evaluated.
 *
 * This is used to implement the `floxpkgs' catalog.
 */
class FloxpkgsFlake : public FloxFlake
{

public:

  const nix::flake::LockedFlake wrappedLockedFlake;

  FloxpkgsFlake( const nix::ref<nix::EvalState> & state,
                 const nix::FlakeRef &            nixpkgsRef );

  [[nodiscard]] nix::ref<nix::eval_cache::EvalCache>
  openEvalCache() override;


}; /* End class `FloxpkgsFlake' */


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
