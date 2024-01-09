/* ========================================================================== *
 *
 * @file flox/buildenv/realise.hh
 *
 * @brief Evaluate an environment definition and realise it.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <map>
#include <optional>
#include <string>
#include <vector>

#include <nix/eval.hh>
#include <nix/store-api.hh>

#include "flox/buildenv/buildenv.hh"
#include "flox/core/exceptions.hh"
#include "flox/resolver/lockfile.hh"
#include <nix/build-result.hh>
#include <nix/flake/flake.hh>
#include <nix/get-drvs.hh>
#include <nix/path-with-outputs.hh>


/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

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
               const System &       system );


/* -------------------------------------------------------------------------- */

/**
 * @brief Create a @a nix::StorePath containing a realised environment.
 * @param pkgs A list of packages to be added to the environment.
 * @param state A `nix` evaluator.
 * @param references A set of indirect dependencies to be added to
 *                   the environment.
 * @param originalPackage A map of packages to be added to the environment.
 * @return A @a nix::StorePath with a realised environment.
 */
const nix::StorePath &
createEnvironmentStorePath(
  std::vector<RealisedPackage> & pkgs,
  nix::EvalState &               state,
  nix::StorePathSet &            references,
  std::map<nix::StorePath, std::pair<std::string, resolver::LockedPackageRaw>> &
    originalPackage );


/* -------------------------------------------------------------------------- */

/**
 * @brief Create a @a nix::StorePath containing a buildscript for a container.
 * @param state A `nix` evaluator.
 * @param environmentStorePath A storepath containing a realised environment.
 * @param system system to build the environment for.
 * @return A @a nix::StorePath to a container builder.
 */
nix::StorePath
createContainerBuilder( nix::EvalState & state,
                        nix::StorePath   environmentStorePath,
                        const System &   system );

/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
