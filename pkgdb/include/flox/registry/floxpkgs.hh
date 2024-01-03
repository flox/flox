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

class FloxpkgsFlake : public FloxFlake
{

  // TODO: modify `ref' to reflect the added rules/pre-processing.
  FloxpkgsFlake( const nix::ref<nix::EvalState> & state,
                 const nix::FlakeRef &            ref );

  /**
   * @brief Open a `nix` evaluator ( with an eval cache when possible ) with the
   *        evaluated `flake` and its outputs in global scope.
   *
   * This will apply any rules/pre-processing to the `flake` before evaluation.
   *
   * @return A `nix` evaluator, potentially with caching.
   */
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
