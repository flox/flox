/* ========================================================================== *
 *
 * @file flox/package.hh
 *
 * @brief Abstract representation of a package.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <functional>
#include <optional>
#include <string>
#include <variant>
#include <vector>

#include <nlohmann/json_fwd.hpp>

#include <nix/eval-cache.hh>
#include <nix/fetchers.hh>
#include <nix/flake/flake.hh>
#include <nix/names.hh>

#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"
#include "versions.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/**
 * @brief Abstract representation of a "package", analogous to a
 *        Nix `derivation'.
 *
 * This abstraction provides a common base for various backends that store,
 * evaluate, and communicate package definitions.
 */
class Package
{

public:

  virtual ~Package()         = default;
  Package()                  = default;
  Package( const Package & ) = default;
  Package( Package && )      = default;

  Package &
  operator=( const Package & )
    = default;
  Package &
  operator=( Package && )
    = default;

  /** @return attribute path where package is defined */
  [[nodiscard]] virtual AttrPath
  getPathStrs() const
    = 0;

  /** @return the derivation `name` field. */
  [[nodiscard]] virtual std::string
  getFullName() const
    = 0;

  /**
   * @return iff the field `pname` is defined then `pname`, otherwise the
   *         `name` field stripped of is _version_ part as recognized by
   *         `nix::DrvName` parsing rules.
   */
  [[nodiscard]] virtual std::string
  getPname() const
    = 0;

  /**
   * @return iff the field `version` is defined then `version`, otherwise the
   *         `name` field stripped of is _pname_ part as recognized by
   *         `nix::DrvName` parsing rules.
   *         If `version` is undefined and `name` contains no version suffix,
   *         then `std::nullopt`.
   */
  [[nodiscard]] virtual std::optional<std::string>
  getVersion() const = 0;

  /** @return The `meta.license.spdxId` field if defined,
   *          otherwise `std::nullopt` */
  [[nodiscard]] virtual std::optional<std::string>
  getLicense() const = 0;

  /** @return The derivation `outputs` list. */
  [[nodiscard]] virtual std::vector<std::string>
  getOutputs() const = 0;

  /**
   * @return The `meta.outputsToInstall` field if defined, otherwise the
   *         derivation `outputs` members to the left of and
   *         including `out`.
   */
  [[nodiscard]] virtual std::vector<std::string>
  getOutputsToInstall() const = 0;

  /** @return The `meta.broken` field if defined, otherwise `std::nullopt`. */
  [[nodiscard]] virtual std::optional<bool>
  isBroken() const = 0;

  /** @return The `meta.unfree` field if defined, otherwise `std::nullopt`. */
  [[nodiscard]] virtual std::optional<bool>
  isUnfree() const = 0;

  /**
   * @return The `meta.description` field if defined,
   * otherwise `std::nullopt`.
   */
  [[nodiscard]] virtual std::optional<std::string>
  getDescription() const = 0;

  /**
   * @return The flake `outputs` subtree the package resides in, being one of
   *         `legacyPackages` or `packages`.
   */
  [[nodiscard]] virtual Subtree
  getSubtreeType() const
  {
    return Subtree( this->getPathStrs().front() );
  }

  /**
   * @return The parsed "package name" prefix of @a this package's
   *         `name` field.
   */
  [[nodiscard]] virtual nix::DrvName
  getParsedDrvName() const
  {
    return { this->getFullName() };
  }

  /**
   * @return `std::nullopt` iff @a this package does not use semantic
   *         versioning, otherwise a normalized semantic version number
   *         coerces from @a this package's `version` information.
   */
  [[nodiscard]] virtual std::optional<std::string>
  getSemver() const
  {
    std::optional<std::string> version = this->getVersion();
    if ( ! version.has_value() ) { return std::nullopt; }
    return versions::coerceSemver( *version );
  }

  /**
   * @brief Create an installable URI string associated with this package
   *        using @a ref as its _input_ part.
   * @param ref Input flake reference associated with @a this package.
   *            This is used to construct the URI on the left side of `#`.
   * @return An installable URI string associated with this package using.
   */
  [[nodiscard]] virtual std::string
  toURIString( const nix::FlakeRef & ref ) const;

  /**
   * @brief Serialize notable package metadata as a JSON object.
   *
   * This may only contains a subset of all available information.
   * @param withDescription Whether to include `description` strings.
   * @return A JSON object with notable package metadata.
   */
  [[nodiscard]] virtual nlohmann::json
  getInfo( bool withDescription = false ) const;


}; /* End class `Package' */


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
