/* ========================================================================== *
 * @file include/flox/env-builder/flox-env.hh
 *
 * @brief Build a flox environment.
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <functional>
#include <map>
#include <string>

#include <nix/builtins/buildenv.hh>
#include <nix/eval.hh>
#include <nix/store-api.hh>

#include "flox/env-builder/buildenv.hh"
#include <flox/resolver/lockfile.hh>


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */


/**
 * @brief Evaluate an environment definition and realise it.
 * @param state A `nix` evaluator.
 * @param lockfile a resolved and locked manifest.
 * @param system system to build the environment for.
 * @return `StorePath` to the environment.
 */
nix::StorePath
createFloxEnv( nix::EvalState &     state,
               resolver::Lockfile & lockfile,
               System &             system );


/* -------------------------------------------------------------------------- */

const nix::StorePath &
createEnvironmentStorePath(
  std::vector<flox::buildenv::Package> & pkgs,
  nix::EvalState &                       state,
  nix::StorePathSet &                    references,
  std::map<nix::StorePath, std::pair<std::string, resolver::LockedPackageRaw>> &
    originalPackage );


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
