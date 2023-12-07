/* ========================================================================== *
 *
 * @file include/flox/env-builder/buildenv.hh
 *
 * @brief Modified version of `nix/builtins/buildenv::buildProfile` customized
 *        for use with `flox`.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <vector>
#include <optional>
#include <string>

#include <nix/derivations.hh>
#include <nix/store-api.hh>


/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

struct Priority
{
  unsigned int               priority;
  std::optional<std::string> parentPath;
  unsigned int               internalPriority;

  Priority() : Priority( 0 ) {}

  Priority( unsigned int priority ) : Priority( priority, {}, 0 ) {}

  Priority( unsigned int               priority,
            std::optional<std::string> parentPath,
            unsigned int               internalPriority )
    : priority { priority }
    , parentPath { parentPath }
    , internalPriority { internalPriority }
  {}
}; /* End struct `Priority' */


/* -------------------------------------------------------------------------- */

struct Package
{
  std::string path;
  bool        active;
  Priority    priority;
  Package( const std::string & path, bool active, Priority priority )
    : path { path }, active { active }, priority { priority }
  {}
};  /* End struct `Package' */


/* -------------------------------------------------------------------------- */

class BuildEnvFileConflictError : public nix::Error
{
public:

  const std::string fileA;
  const std::string fileB;
  int        priority;

  BuildEnvFileConflictError( const std::string fileA, const std::string fileB, int priority )
    : nix::Error(
      "Unable to build profile. There is a conflict for the following files:\n"
      "\n"
      "  %1%\n"
      "  %2%",
      fileA,
      fileB )
    , fileA( fileA )
    , fileB( fileB )
    , priority( priority )
  {}
};  /* End class `BuildEnvFileConflictError' */


/* -------------------------------------------------------------------------- */

/** @brief Modified version of `nix/builtins/buildenv::buildProfile` that has
 * special handling for flox packages.
 * @param out the path to a build directory. (This directory will be loaded
 * into the store by the caller)
 * @param pkgs a list of packages to include in the build environment.
 */
void
buildEnvironment( const std::string & out, std::vector<Package> && pkgs );


/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
