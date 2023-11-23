#pragma once

#include <nix/store-api.hh>

#include <flox/resolver/lockfile.hh>

/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */


/**
 * Evaluate an environment definition and realise it.
 * @param state A `nix` evaluator.
 * @param lockfile a resolved and locked manifest.
 * @param system system to build the environment for.
 * @return `StorePath` to the environment.
 */
nix::StorePath
createFloxEnv( nix::EvalState &          state,
               resolver::Lockfile & lockfile,
               System &             system );

const nix::StorePath &
createEnvironmentStorePath( nix::Packages &     pkgs,
                            nix::EvalState &    state,
                            nix::StorePathSet & references );

}  // namespace flox
