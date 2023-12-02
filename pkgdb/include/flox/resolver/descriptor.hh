/* ========================================================================== *
 *
 * @file flox/resolver/descriptor.hh
 *
 * @brief A set of user inputs used to set input preferences and query
 *        parameters during resolution.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <functional>
#include <string>
#include <vector>

#include <nlohmann/json.hpp>

#include "flox/core/types.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/registry.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

/** @brief A named group which a descriptor/package can be a member of. */
using GroupName = std::string;


/* -------------------------------------------------------------------------- */

/**
 * @brief A set of user defined requirements describing a package/dependency.
 *
 * This _raw_ struct is defined to generate parsers.
 * The _real_ form is @a flox::resolver::ManifestDescriptor.
 */
struct ManifestDescriptorRaw
{

public:

  /** The delimiter for providing an input when the descriptor is a string */
  static const auto inputSigil = ':';

  /** The delimiter for specifying a version when the descriptor is a string */
  static const auto versionSigil = '@';

  /** The signifier that the version should be treated exactly
   *  i.e. not a semver range
   */
  static const auto exactVersionSigil = '=';

  /**
   * Match `name`, `pname`, or `attrName`.
   * Maps to `flox::pkgdb::PkgQueryArgs::pnameOrAttrName`.
   */
  std::optional<std::string> name;

  /**
   * Match `version` or `semver` if a modifier is present.
   *
   * Strings beginning with an `=` will filter by exact match on `version`.
   * Any string which may be interpreted as a semantic version range will
   * filter on the `semver` field.
   * All other strings will filter by exact match on `version`.
   */
  std::optional<std::string> version;

  /** @brief A dot separated attribut path, or list representation. */
  using Path = std::variant<std::string, flox::AttrPath>;
  /** Match a relative path. */
  std::optional<Path> path;

  /**
   * @brief A dot separated attribut path, or list representation.
   *        May contain `null` members to represent _globs_.
   *
   * NOTE: `AttrPathGlob` is a `std::vector<std::optional<std::string>>`
   *       which represnts an absolute attribute path which may have
           `std::nullopt` as its second element to avoid indicating a
           particular system.
   */
  using AbsPath = std::variant<std::string, AttrPathGlob>;
  /** Match an absolute path, allowing globs for `system`. */
  std::optional<AbsPath> absPath;

  /** Only resolve for a given set of systems. */
  std::optional<std::vector<System>> systems;

  /** Whether resoution is allowed to fail without producing errors. */
  std::optional<bool> optional;

  // TODO: Not implemented.
  /** Named _group_ that the package is a member of. */
  std::optional<GroupName> packageGroup;

  // TODO: Not implemented.
  /** Force resolution is the named input or _flake reference_. */
  std::optional<std::variant<std::string, nix::fetchers::Attrs>>
    packageRepository;


  /**
   * Rank a package's priority for handling conflicting files.
   * The default value is `5` ( set in @a flox::resolver::ManifestDescriptor ).
   *
   * Packages with higher @a priority values will take precendence over those
   * with lower @a priority values.
   */
  std::optional<unsigned> priority;

  /**
   * @brief Ensure that a raw descriptor's fields are valid or
   *        throws an exception if the descriptor is invalid.
   *
   * This requires that the `abspath` field is valid, and consistent with
   * `path` and/or `systems` fields if they are set.
   */
  void
  check( const std::string iid = "*" ) const;

  /** @brief Reset to default/empty state. */
  void
  clear();

  ManifestDescriptorRaw() = default;

  explicit ManifestDescriptorRaw( const std::string_view & descriptor );

}; /* End struct `ManifestDescriptorRaw' */


/* -------------------------------------------------------------------------- */

// TODO: support `packageRepository' field
/**
 * @brief Convert a JSON object to an @a flox::ManifestDescriptorRaw. */
void
from_json( const nlohmann::json & jfrom, ManifestDescriptorRaw & descriptor );

/**
 * @brief Convert an @a flox::resolver::ManifestDescriptorRaw to a
 *              JSON object.
 */
void
to_json( nlohmann::json & jto, const ManifestDescriptorRaw & descriptor );


/* -------------------------------------------------------------------------- */

/**
 * @class flox::pkgdb::ParseManifestDescriptorRawException
 * @brief An exception thrown when parsing @a
 * flox::resolver::ManifestDescriptorRaw from JSON.
 * @{
 */
FLOX_DEFINE_EXCEPTION( ParseManifestDescriptorRawException,
                       EC_PARSE_MANIFEST_DESCRIPTOR_RAW,
                       "error parsing manifest descriptor" )
/** @} */


/* -------------------------------------------------------------------------- */

/**
 * @brief Validates a single attribute name, `pname`, etc from a globbed
 * @a flox::AttrPathGlob, returning the attribute name if it is suitable for
 * appearing as a single attribute in a descriptor.
 */
std::optional<std::string>
validatedSingleAttr( const AttrPathGlob & attrs );

/**
 * @brief Returns true if any component in the attribute path contains a glob
 * but is not itself entirely a glob. For example, this would return `true` for
 * `foo.b*ar.baz`, but not for `foo.*.baz` since `b*ar` contains a glob, but is
 * not itself entirely a glob.
 */
bool
globInAttrName( const AttrPathGlob & attrs );

/**
 * @brief Validates a relative attribute path from a globbed
 * @a flox::AttrPathGlob, returning the string form of the relative path for
 * use in the @a flox::resolver::ManifestDescriptorRaw.
 */
std::vector<std::string>
validatedRelativePath( const AttrPathGlob &             attrs,
                       const std::vector<std::string> & strings );

/**
 * @brief Validates an absolute path from a globbed
 * @a flox::AttrPathGlob, returning the attribute path if it is suitable for
 * an absolute path appearing in a descriptor.
 */
AttrPathGlob
validatedAbsolutePath( const AttrPathGlob & attrs );

/**
 * @brief Returns `true` if the attribute path has enough path components and
 * begins with one of the allowed prefixes (`legacyPackages` or `packages`).
 */
bool
isAbsolutePath( const AttrPathGlob & attrs );

/* -------------------------------------------------------------------------- */

/**
 * @brief A set of user defined requirements describing a package/dependency.
 *
 * May either be defined as a set of attributes or with a string matching
 * this syntax:
 * `[<input>:]((<attr>.)+<attrName>)|(<pname>|<attrName>|<name>)[@(<semver>|=<version>)]`
 */
struct ManifestDescriptor
{

public:

  /** Match `name`, `pname`, or `attrName` */
  std::optional<std::string> name;

  /** Whether resolution is allowed to fail without producing errors. */
  bool optional = false;

  /** Named _group_ that the package is a member of. */
  std::optional<GroupName> group;

  /** Match `version`. */
  std::optional<std::string> version;

  /** Match a semantic version range. */
  std::optional<std::string> semver;

  /** Match a subtree. */
  std::optional<Subtree> subtree;

  /** Only resolve for a given set of systems. */
  std::optional<std::vector<System>> systems;

  /** Match a relative attribute path. */
  std::optional<flox::AttrPath> path;

  /** Force resolution in a given input, _flake reference_. */
  std::optional<nix::FlakeRef> input;

  /**
   * Rank a package's priority for handling conflicting files.
   * The default value is `5` ( set in @a flox::resolver::ManifestDescriptor ).
   *
   * Packages with higher @a priority values will take precendence over those
   * with lower @a priority values.
   */
  unsigned priority = 5;


  ManifestDescriptor() = default;

  explicit ManifestDescriptor( const std::string_view & descriptor )
    : ManifestDescriptor( ManifestDescriptorRaw( descriptor ) ) {};

  explicit ManifestDescriptor( const ManifestDescriptorRaw & raw );

  explicit ManifestDescriptor( std::string_view              installID,
                               const ManifestDescriptorRaw & raw )
    : ManifestDescriptor( raw )
  {
    if ( ! this->name.has_value() ) { this->name = installID; }
  }

  /**
   * @brief Ensure that a descriptor has at least `name`, `path`, or
   *        `absPath` fields.
   *        Throws an exception if the descriptor is invalid.
   */
  void
  check() const;

  /** @brief Reset to default state. */
  void
  clear();

  /**
   * @brief Fill a @a flox::pkgdb::PkgQueryArgs struct with preferences to
   *        lookup packages.
   *
   * NOTE: This DOES NOT clear @a pqa before filling it.
   * This is intended to be used after filling @a pqa with global preferences.
   * @param pqa A set of query args to _fill_ with preferences.
   * @return A reference to the modified query args.
   */
  pkgdb::PkgQueryArgs &
  fillPkgQueryArgs( pkgdb::PkgQueryArgs & pqa ) const;


}; /* End struct `ManifestDescriptor' */


/* -------------------------------------------------------------------------- */

/**
 * @brief Convert an @a flox::resolver::ManifestDescriptor to a
 *              JSON object.
 */
void
to_json( nlohmann::json & jto, const ManifestDescriptor & descriptor );

/* -------------------------------------------------------------------------- */

/**
 * @class flox::resolver::InvalidManifestDescriptorException
 * @brief An exception thrown when a package descriptor in a manifest
 *        is invalid.
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( InvalidManifestDescriptorException,
                       EC_INVALID_MANIFEST_DESCRIPTOR,
                       "invalid manifest descriptor" )
/** @} */


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
