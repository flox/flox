#pragma once
///@file

#include <nix/derivations.hh>
#include <nix/store-api.hh>

namespace flox::buildenv {
using namespace nix;

struct Priority
{
  unsigned int        priority;
  std::optional<Path> parentPath;
  unsigned int        internalPriority;

  Priority() : Priority( 0 ) {}

  Priority( unsigned int priority ) : Priority( priority, {}, 0 ) {}

  Priority( unsigned int        priority,
            std::optional<Path> parentPath,
            unsigned int        internalPriority )
    : priority { priority }
    , parentPath { parentPath }
    , internalPriority { internalPriority }
  {}
};

struct Package
{
  Path     path;
  bool     active;
  Priority priority;
  Package( const Path & path, bool active, Priority priority )
    : path { path }, active { active }, priority { priority }
  {}
};

class BuildEnvFileConflictError : public Error
{
public:

  const Path fileA;
  const Path fileB;
  int        priority;

  BuildEnvFileConflictError( const Path fileA, const Path fileB, int priority )
    : Error(
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
};

typedef std::vector<Package> Packages;

/** @brief Modified version of `nix/builtins/buildenv::buildProfile` that has
 * special handling for flox packages.
 * @param out the path to a build directory. (This directory will be loaded
 * into the store by the caller)
 * @param pkgs a list of packages to include in the build environment.
 */
void
buildEnvironment( const Path & out, Packages && pkgs );


}  // namespace flox::buildenv
