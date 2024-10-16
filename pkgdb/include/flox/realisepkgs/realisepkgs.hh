/* ========================================================================== *
 *
 * @file flox/realisepkgs/realise.hh
 *
 * @brief Compose packages and handle conflicts.
 *        Modified version of `nix/builtins/realisepkgs`
 *        that has special handling for flox packages.
 *
 *
 * -------------------------------------------------------------------------- */

#include <optional>
#include <utility>
#include <vector>


#include "flox/core/exceptions.hh"

/* -------------------------------------------------------------------------- */

namespace flox::realisepkgs {

struct RealisedPackage
{
  std::string path;
  bool        active {};

  ~RealisedPackage()                         = default;
  RealisedPackage()                          = default;
  RealisedPackage( const RealisedPackage & ) = default;
  RealisedPackage( RealisedPackage && )      = default;

  explicit RealisedPackage( std::string path, bool active = false )
    : path( std::move( path ) ), active( active )
  {}

  RealisedPackage &
  operator=( const RealisedPackage & )
    = default;
  RealisedPackage &
  operator=( RealisedPackage && )
    = default;

}; /* End struct `RealisedPackage' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::realisepkgs

/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
