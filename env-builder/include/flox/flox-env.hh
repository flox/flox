#pragma once

#include "flox/buildenv.hh"

#include <nix/store-api.hh>

#include <flox/resolver/lockfile.hh>
#include <nix/builtins/buildenv.hh>
#include <nix/eval.hh>


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
createFloxEnv( nix::EvalState &     state,
               resolver::Lockfile & lockfile,
               System &             system );

const nix::StorePath &
createEnvironmentStorePath(
  flox::buildenv::Packages & pkgs,
  nix::EvalState &           state,
  nix::StorePathSet &        references,
  std::map<nix::StorePath, std::pair<std::string, resolver::LockedPackageRaw>> &
    originalPackage );

/* -------------------------------------------------------------------------- */

}  // namespace flox
