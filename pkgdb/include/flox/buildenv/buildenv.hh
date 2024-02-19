/* ========================================================================== *
 *
 * @file flox/buildenv/realise.hh
 *
 * @brief Compose packages and handle conflicts.
 *        Modified version of `nix/builtins/buildenv`
 *        that has special handling for flox packages.
 *
 *
 * -------------------------------------------------------------------------- */

#include <optional>
#include <utility>
#include <vector>


#include "flox/core/exceptions.hh"

/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

struct Priority
{
  unsigned                   priority {};
  std::optional<std::string> parentPath;
  unsigned                   internalPriority {};

  ~Priority()                  = default;
  Priority()                   = default;
  Priority( const Priority & ) = default;
  Priority( Priority && )      = default;

  explicit Priority( unsigned                   priority,
                     std::optional<std::string> parentPath       = std::nullopt,
                     unsigned                   internalPriority = 0 )
    : priority( priority )
    , parentPath( std::move( parentPath ) )
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
  bool        active {};
  Priority    priority;

  ~RealisedPackage()                         = default;
  RealisedPackage()                          = default;
  RealisedPackage( const RealisedPackage & ) = default;
  RealisedPackage( RealisedPackage && )      = default;

  explicit RealisedPackage( std::string path,
                            bool        active   = false,
                            Priority    priority = {} )
    : path( std::move( path ) )
    , active( active )
    , priority( std::move( priority ) )
  {}

  RealisedPackage &
  operator=( const RealisedPackage & )
    = default;
  RealisedPackage &
  operator=( RealisedPackage && )
    = default;

}; /* End struct `RealisedPackage' */


/* -------------------------------------------------------------------------- */

/** @brief A conflict between two files with the same priority.
 *
 * This exception is thrown when we attempt to build an environment with two
 * store paths with the same priority that contain the same file.
 *
 * This exception is intended to be caught by the caller and converted into a
 * @a PackageConflict which restores the originating packages for display
 * purposes
 */
class FileConflict : public std::exception
{

public:

  std::string  fileA;
  std::string  fileB;
  unsigned int priority;

  FileConflict( std::string fileA, std::string fileB, unsigned int priority )
    : fileA( std::move( fileA ) )
    , fileB( std::move( fileB ) )
    , priority( priority )
  {}
}; /* End class `FileConflict' */


/* -------------------------------------------------------------------------- */

/** @brief Modified version of `nix/builtins/buildenv::buildProfile` that has
 *         special handling for flox packages.
 * @param out the path to a build directory.
 *            ( This directory will be loaded into the store by the caller )
 * @param pkgs a list of packages to include in the build environment.
 */
void
buildEnvironment( const std::string &            out,
                  std::vector<RealisedPackage> & pkgs );

/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv

/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
