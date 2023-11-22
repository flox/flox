/* ========================================================================== *
 *
 * @file nix/run.cc
 *
 * @brief Helpers used to construct environments.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include "nix/get-drvs.hh"

/* -------------------------------------------------------------------------- */

namespace nix {

/* -------------------------------------------------------------------------- */

/**
 * Lookup information about derivations in an environment.
 * @param state A `nix` evaluator.
 * @param userEnv Path to the environment which should be queried.
 * @return A list of `derivation` metadata associated with installed packages.
 */
DrvInfos
queryInstalled( EvalState & state, const Path & userEnv );


/**
 * Evaluate an environment definition and realise it.
 * @param state A `nix` evaluator.
 * @param elems Derivations to install into the environment.
 * @param profile Path to target environment.
 * @param keepDerivations Whether to preserve installable recipes for
 *                        installables so that they may be shared.
 * @param lockToken Unique identifier associated with the environment.
 *                  This is used to sync multiple processes attempting to modify
 *                  the environment's lockfile.
 * @return `true` iff the environment is created successfully;
 *         `false` if an error was encountered.
 */
bool
createUserEnv( EvalState &         state,
               DrvInfos &          elems,
               const Path &        profile,
               bool                keepDerivations,
               const std::string & lockToken );

}  // namespace nix


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
