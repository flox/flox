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
#include <vector>

#include "flox/core/exceptions.hh"


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


  ~RealisedPackage()                         = default;
  RealisedPackage()                          = default;
  RealisedPackage( const RealisedPackage & ) = default;
  RealisedPackage( RealisedPackage && )      = default;

  explicit RealisedPackage( std::string path,
                            bool        active   = false,
                            Priority    priority = {} )
    : path( path ), active( active ), priority( priority )
  {}

  RealisedPackage &
  operator=( const RealisedPackage & )
    = default;
  RealisedPackage &
  operator=( RealisedPackage && )
    = default;


}; /* End struct `RealisedPackage' */


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

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
