/* ========================================================================== *
 *
 * @file flox/buildenv/buildenv-lockfile.hh
 *
 * @brief The subset of a lockfile that buildenv needs in order to build an
 *        environment.
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include "flox/core/types.hh"
#include "flox/resolver/lockfile.hh"
#include "flox/resolver/manifest-raw.hh"

/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

/** @brief The components of a package that buildenv needs to realise it. */
struct BuildenvLockedPackage
{
  std::string system;
  std::string installId;
  // TODO: this could probably just be attrs
  resolver::LockedInputRaw input;
  AttrPath                 attrPath;
  unsigned                 priority;
};


/* -------------------------------------------------------------------------- */

struct BuildenvLockfile
{
  // TODO: we don't need the packages inside the manifest
  resolver::ManifestRaw              manifest;
  std::vector<BuildenvLockedPackage> packages;

  /** @brief Loads a JSON object to @a flox::buildenv::BuildenvLockfile
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
   *         @a flox::buildenv::BuildenvLockfile assuming the content is a V0
   *         lockfile.
   * */
  void
  from_v0_content( const nlohmann::json & jfrom );

  /** @brief Helper to convert a JSON object to a
   *         @a flox::buildenv::BuildenvLockfile assuming the content is a V1
   *         lockfile.
   * */
  void
  from_v1_content( const nlohmann::json & jfrom );
};


/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
