/* ========================================================================== *
 *
 * @file flox/realisepkgs/realisepkgs-lockfile.hh
 *
 * @brief The subset of a lockfile that realisepkgs needs in order to build an
 *        environment.
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include "flox/core/types.hh"
#include "flox/resolver/lockfile.hh"
#include "flox/resolver/manifest-raw.hh"

/* -------------------------------------------------------------------------- */

namespace flox::realisepkgs {

/* -------------------------------------------------------------------------- */

/** @brief The components of a package that realisepkgs needs to realise it. */
struct RealisepkgsLockedPackage
{
  std::string system;
  std::string installId;
  // TODO: this could probably just be attrs
  resolver::LockedInputRaw input;
  AttrPath                 attrPath;
  unsigned                 priority;
};


/* -------------------------------------------------------------------------- */

struct RealisepkgsLockfile
{
  // TODO: we don't need the packages inside the manifest
  resolver::ManifestRaw                 manifest;
  std::vector<RealisepkgsLockedPackage> packages;

  /** @brief Loads a JSON object to @a flox::realisepkgs::RealisepkgsLockfile
   *
   * The JSON object can be either a V0 or V1 lockfile, which is read from the
   * `lockfile-version` field.
   *
   * Differences between different types of descriptors are handled here:
   * - attr_path is defaulted
   * - inputs are transformed to flox-nixpkgs inputs
   * */
  void
  load_from_content( const nlohmann::json & jfrom );

  /** @brief Helper to convert a JSON object to a
   *         @a flox::realisepkgs::RealisepkgsLockfile assuming the content is a
   * V0 lockfile.
   * */
  void
  from_v0_content( const nlohmann::json & jfrom );

  /** @brief Helper to convert a JSON object to a
   *         @a flox::realisepkgs::RealisepkgsLockfile assuming the content is a
   * V1 lockfile.
   * */
  void
  from_v1_content( const nlohmann::json & jfrom );
};


/* -------------------------------------------------------------------------- */

}  // namespace flox::realisepkgs


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
