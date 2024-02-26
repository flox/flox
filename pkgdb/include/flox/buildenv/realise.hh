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

/**
 * @class flox::buildenv::SystemNotSupportedByLockfile
 * @brief An exception thrown when a lockfile is is missing a package.<system>
 * entry fro the requested system.
 * @{
 */
FLOX_DEFINE_EXCEPTION( SystemNotSupportedByLockfile,
                       EC_LOCKFILE_INCOMPATIBLE_SYSTEM,
                       "unsupported system" )
/** @} */


/**
 * @class flox::buildenv::PackageConflictException
 * @brief An exception thrown when two packages conflict.
 * I.e. the same file path is found in two different packages with the same
 * priority.
 * @{
 */
FLOX_DEFINE_EXCEPTION( PackageConflictException,
                       EC_BUILDENV_CONFLICT,
                       "conflicting packages" )
/** @} */


/**
 * @class flox::buildenv::PackageUnsupportedSystem
 * @brief An exception thrown when a package fails to evaluate,
 * because the system is not supported.
 * @{
 */
FLOX_DEFINE_EXCEPTION( PackageUnsupportedSystem,
                       EC_PACKAGE_EVAL_INCOMPATIBLE_SYSTEM,
                       "system unsupported by package" )
/** @} */

/**
 * @class flox::buildenv::PackageEvalFailure
 * @brief An exception thrown when a package fails to evaluate.
 * @{
 */
FLOX_DEFINE_EXCEPTION( PackageEvalFailure,
                       EC_PACKAGE_EVAL_FAILURE,
                       "general package eval failure" )
/** @} */


/**
 * @class flox::buildenv::PackageBuildFailure
 * @brief An exception thrown when a package fails to build.
 * @{
 */
FLOX_DEFINE_EXCEPTION( PackageBuildFailure,
                       EC_PACKAGE_BUILD_FAILURE,
                       "build failure" )
/** @} */


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
 * @brief Merge all components of the environment into a single store path.
 * @param state Nix state.
 * @param pkgs List of packages to include in the environment.
 *             - outputs of packages declared in the environment manifest
 *             - flox specific packages (activation scripts, profile.d, etc.)
 * @param references Set of store paths that the environment depends on.
 * @param originalPackage Map of store paths to the locked package definition
 *                        that provided them.
 * @return The combined store path of the environment.
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
createContainerBuilder( nix::EvalState &       state,
                        const nix::StorePath & environmentStorePath,
                        const System &         system );

/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
