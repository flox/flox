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

#include <nix/builtins/buildenv.hh>
#include <nix/eval.hh>
#include <nix/store-api.hh>

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

  ~Priority()                  = default;
  Priority()                   = default;
  Priority( const Priority & ) = default;
  Priority( Priority && )      = default;

  explicit Priority( unsigned                   priority,
                     std::optional<std::string> parentPath       = std::nullopt,
                     unsigned                   internalPriority = 0 )
    : priority( priority )
    , parentPath( parentPath )
    , internalPriority( internalPriority )
  {}

  Priority &
  operator=( const Priority & )
    = default;
  Priority &
  operator=( Priority && )
    = default;

}; /* End struct `Priority' */


/* -------------------------------------------------------------------------- */

struct RealisedPackage
{
  std::string path;
  bool        active;
  Priority    priority;
}; /* End struct `Package' */


/* -------------------------------------------------------------------------- */

/** @brief A conflict between two files with the same priority. */
class BuildEnvFileConflictError : public FloxException
{

private:

  const std::string fileA;
  const std::string fileB;
  const int         priority;


public:

  BuildEnvFileConflictError( const std::string fileA,
                             const std::string fileB,
                             int               priority )
    : FloxException(
      "buildenv file conflict",
      nix::fmt(
        "there is a conflict for the files with priority %zu: `%s' and `%s'",
        priority,
        fileA,
        fileB ) )
    , fileA( fileA )
    , fileB( fileB )
    , priority( priority )
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

  const std::string &
  getFileA() const
  {
    return this->fileA;
  }

  const std::string &
  getFileB() const
  {
    return this->fileB;
  }

  int
  getPriority() const
  {
    return this->priority;
  }


}; /* End class `BuildEnvFileConflictError' */


/* -------------------------------------------------------------------------- */

/** @brief Modified version of `nix/builtins/buildenv::buildProfile` that has
 *         special handling for flox packages.
 * @param out the path to a build directory.
 *            ( This directory will be loaded into the store by the caller )
 * @param pkgs a list of packages to include in the build environment.
 */
void
buildEnvironment( const std::string &             out,
                  std::vector<RealisedPackage> && pkgs );


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

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
