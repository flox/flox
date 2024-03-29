/* ========================================================================== *
 *
 * @file flox/raw-package.hh
 *
 * @brief The simplest `Package' implementation comprised of raw values.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include <nlohmann/json.hpp>

#include "flox/core/types.hh"
#include "flox/package.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/**
 * @brief The simplest `Package' implementation comprised of raw values.
 *
 * This form largely exists for testing purposes.
 */
class RawPackage : public Package
{

public:

  AttrPath                   path;
  std::string                name;
  std::string                pname;
  std::optional<std::string> version;
  std::optional<std::string> semver;
  std::optional<std::string> license;
  std::vector<std::string>   outputs;
  std::vector<std::string>   outputsToInstall;
  std::optional<bool>        broken;
  std::optional<bool>        unfree;
  std::optional<std::string> description;

  explicit RawPackage( AttrPath                         path    = {},
                       std::string_view                 name    = {},
                       std::string_view                 pname   = {},
                       std::optional<std::string>       version = std::nullopt,
                       std::optional<std::string>       semver  = std::nullopt,
                       std::optional<std::string>       license = std::nullopt,
                       const std::vector<std::string> & outputs = { "out" },
                       const std::vector<std::string> & outputsToInstall
                       = { "out" },
                       std::optional<bool>        broken      = std::nullopt,
                       std::optional<bool>        unfree      = std::nullopt,
                       std::optional<std::string> description = std::nullopt )
    : path( std::move( path ) )
    , name( name )
    , pname( pname )
    , version( std::move( version ) )
    , semver( std::move( semver ) )
    , license( std::move( license ) )
    , outputs( outputs )
    , outputsToInstall( outputsToInstall )
    , broken( broken )
    , unfree( unfree )
    , description( std::move( description ) )
  {}


  /* --------------------------------------------------------------------------
   */

  [[nodiscard]] AttrPath
  getPathStrs() const override
  {
    return this->path;
  }

  [[nodiscard]] std::string
  getFullName() const override
  {
    return this->name;
  }

  [[nodiscard]] std::string
  getPname() const override
  {
    return this->pname;
  }

  [[nodiscard]] std::optional<std::string>
  getVersion() const override
  {
    return this->version;
  }

  [[nodiscard]] std::optional<std::string>
  getSemver() const override
  {
    return this->semver;
  }

  [[nodiscard]] std::optional<std::string>
  getLicense() const override
  {
    return this->license;
  }

  [[nodiscard]] std::vector<std::string>
  getOutputs() const override
  {
    return this->outputs;
  }

  [[nodiscard]] std::vector<std::string>
  getOutputsToInstall() const override
  {
    return this->outputsToInstall;
  }

  [[nodiscard]] std::optional<bool>
  isBroken() const override
  {
    return this->broken;
  }

  [[nodiscard]] std::optional<bool>
  isUnfree() const override
  {
    return this->unfree;
  }

  [[nodiscard]] std::optional<std::string>
  getDescription() const override
  {
    return this->description;
  }


}; /* End class `RawPackage' */


/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::RawPackage. */
void
from_json( const nlohmann::json & jfrom, RawPackage & pkg );

/** @brief Convert a @a flox::RawPackage to a JSON object. */
void
to_json( nlohmann::json & jto, const flox::RawPackage & pkg );


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
