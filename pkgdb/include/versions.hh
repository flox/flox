/* ========================================================================== *
 *
 * @file versions.hh
 *
 * @brief Interfaces used to perform version number analysis, especially
 *        _Semantic Version_ processing.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <list>
#include <nix/util.hh>
#include <optional>
#include <regex>
#include <string>


/* -------------------------------------------------------------------------- */

/** @brief Interfaces for analyzing version numbers. */
namespace versions {

/* -------------------------------------------------------------------------- */

/**
 * @brief Typed exception wrapper used for version parsing/comparison errors.
 */
class VersionException : public std::exception
{
private:

  std::string msg;

public:

  VersionException( std::string_view msg ) : msg( msg ) {}
  const char *
  what() const noexcept override
  {
    return this->msg.c_str();
  }
};


/* -------------------------------------------------------------------------- */

/** @return `true` iff @a version is a valid _semantic version_ string. */
bool
isSemver( const std::string & version );

/** @return `true` iff @a version is a _datestamp-like_ version string. */
bool
isDate( const std::string & version );

/** @return `true` iff @a version can be interpreted as _semantic version_. */
bool
isCoercibleToSemver( const std::string & version );

/**
 * @brief Determine if @a version is a valid _semantic version range_ string.
 *
 * This is far from a complete check, but it should be sufficient for our usage.
 * This essentially checks that the first token of the string is a valid range,
 * a `4.2.0 - 5.3.1` style range, or a special token.
 * ( See expanded discussion below for futher details ).
 *
 * Leading and trailing space is ignored.
 *
 * This will count _exact version matches_ such as `4.2.0` as _ranges_.
 *
 * This will count _the empty string_ ( `""` ), `*`,  `any`, and `latest`
 * as ranges ( aligning with `node-semver` ).
 *
 *
 * Limitations:
 * This covers the 99% case to distinguish between a range and "static" version.
 * The main reason to detect this is because from the CLI we can't immediately
 * tell whether `<NAME>@<VERSION-OR-SEMVER>` is an exact version match
 * ( like a date ), or a real range.
 * This does a "best effort" detection which is suitable for our purposes today.
 *
 * @return `true` iff @a version is a valid _semantic version range_ string.
 *
 * @see flox::resolver::ManifestDescriptor::semver
 */
bool
isSemverRange( const std::string & version );


/* -------------------------------------------------------------------------- */

/**
 * @brief Attempt to coerce strings such as `"v1.0.2"` or `1.0` to valid
 *        semantic version strings.
 *
 * @return `std::nullopt` iff @a version cannot be interpreted as
 *          _semantic version_.
 *          A valid semantic version string otherwise.
 */
std::optional<std::string>
coerceSemver( std::string_view version );


/* -------------------------------------------------------------------------- */

/**
 * @brief Invokes `node-semver` by `exec`.
 *
 * @param args List of arguments to pass to `semver` executable.
 * @return Pair of error-code and output string.
 */
std::pair<int, std::string>
runSemver( const std::list<std::string> & args );

/**
 * @brief Filter a list of versions by a `node-semver` _semantic version range_.
 *
 * @param range A _semantic version range_ as taken by `node-semver`.
 * @param versions A list of _semantic versions_ to filter.
 * @return The list of _semantic versions_ from @a versions which fall in the
 *         range specified by @a range.
 */
std::list<std::string>
semverSat( const std::string & range, const std::list<std::string> & versions );


/* -------------------------------------------------------------------------- */

}  // namespace versions


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
