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

#include <nix/store-api.hh>
#include <nix/builtins/buildenv.hh>
#include <nix/eval.hh>
#include <nix/util.hh>

#include "flox/core/exceptions.hh"
#include "flox/resolver/lockfile.hh"


/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

struct Priority
{
  unsigned                   priority;
  std::optional<std::string> parentPath;
  unsigned                   internalPriority;
};  /* End struct `Priority' */


/* -------------------------------------------------------------------------- */

struct Package
{
  std::string path;
  bool        active;
  Priority    priority;
};  /* End struct `Package' */


/* -------------------------------------------------------------------------- */

/** @brief A conflict between two files with the same priority. */
class BuildEnvFileConflictError : public FloxException
{

public:

  BuildEnvFileConflictError( const std::string fileA,
                             const std::string fileB,
                             int priority )
    : FloxException(
        "buildenv file conflict",
        nix::fmt(
          "there is a conflict for the files with priority %zu: `%s' and `%s'"
        ),
        priority,
        fileA,
        fileB
      )
    {}

  [[nodiscard]] error_category
  getErrorCode() const noexcept override
  {
    return EC_BUILDENV_CONFLICT;
  }

  [[nodiscard]] std::string_view
  getCategoryMessage() const noexcept override
  {
    return "buildenv file conflict";
  }


};  /* End class `BuildEnvFileConflictError' */


/* -------------------------------------------------------------------------- */

/** @brief Modified version of `nix/builtins/buildenv::buildProfile` that has
 * special handling for flox packages.
 * @param out the path to a build directory. (This directory will be loaded
 * into the store by the caller)
 * @param pkgs a list of packages to include in the build environment.
 */
void
buildEnvironment( const std::string & out, std::vector<Package> & pkgs );


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
  flox::buildenv::std::vector<Package> & pkgs,
  nix::EvalState &           state,
  nix::StorePathSet &        references,
  std::map<nix::StorePath, std::pair<std::string, resolver::LockedPackageRaw>> &
    originalPackage );


/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
