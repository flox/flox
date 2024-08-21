/* ========================================================================== *
 *
 * @file resolver/lockfile.cc
 *
 * @brief A lockfile representing a resolved environment.
 *
 * This lockfile is processed by `mkEnv` to realize an environment.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>

#include <nix/attrs.hh>
#include <nix/hash.hh>

#include "flox/core/util.hh"
#include "flox/resolver/lockfile.hh"
#include "flox/resolver/manifest-raw.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

void
LockfileRaw::check() const
{
  if ( this->lockfileVersion != 0 )
    {
      throw InvalidLockfileException(
        "unsupported lockfile version "
        + std::to_string( this->lockfileVersion ) );
    }
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, LockedInputRaw & raw )
{
  assertIsJSONObject<InvalidLockfileException>( jfrom, "locked input" );

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "fingerprint" )
        { /* obsolete field */
        }
      else if ( key == "url" )
        {
          try
            {
              value.get_to( raw.url );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidLockfileException(
                "couldn't parse locked input field '" + key + "'",
                extract_json_errmsg( err ) );
            }
        }
      else if ( key == "attrs" )
        {
          try
            {
              value.get_to( raw.attrs );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidLockfileException(
                "couldn't parse locked input field '" + key + "'",
                extract_json_errmsg( err ) );
            }
        }
      else
        {
          throw InvalidLockfileException( "encountered unexpected field '" + key
                                          + "' while parsing locked input" );
        }
    }
}


void
to_json( nlohmann::json & jto, const LockedInputRaw & raw )
{
  jto = { { "url", raw.url }, { "attrs", raw.attrs } };
}


/* -------------------------------------------------------------------------- */

std::ostream &
operator<<( std::ostream & oss, const LockedInputRaw & raw )
{
  return oss << nlohmann::json( raw ).dump();
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, LockedPackageRaw & raw )
{
  assertIsJSONObject<InvalidLockfileException>( jfrom, "locked package" );

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "input" )
        {
          try
            {
              value.get_to( raw.input );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidLockfileException(
                "couldn't parse package input field '" + key + "'",
                extract_json_errmsg( err ) );
            }
        }
      else if ( key == "attr-path" )
        {
          try
            {
              value.get_to( raw.attrPath );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidLockfileException(
                "couldn't parse package input field '" + key + "'",
                extract_json_errmsg( err ) );
            }
        }
      else if ( key == "priority" )
        {
          try
            {
              value.get_to( raw.priority );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidLockfileException(
                "couldn't parse package input field '" + key + "'",
                extract_json_errmsg( err ) );
            }
        }
      else if ( key == "info" ) { raw.info = value; }
      else
        {
          throw InvalidLockfileException( "encountered unexpected field '" + key
                                          + "' while parsing locked package" );
        }
    }
}


void
to_json( nlohmann::json & jto, const LockedPackageRaw & raw )
{
  jto = { { "input", raw.input },
          { "attr-path", raw.attrPath },
          { "priority", raw.priority },
          { "info", raw.info } };
}


/* -------------------------------------------------------------------------- */

std::ostream &
operator<<( std::ostream & oss, const LockedPackageRaw & raw )
{
  return oss << nlohmann::json( raw ).dump();
}


/* -------------------------------------------------------------------------- */

std::vector<CheckPackageWarning>
LockedPackageRaw::check( const std::string &     packageId,
                         const Options::Allows & allows ) const
{
  std::vector<CheckPackageWarning> result;

  /**
   * Defensively assume the package _is_ unfree, if field is missing.
   * By default unfree packages are allowed,
   * but if denied we should prevent false negatives.
   */
  bool unfree = this->info.value( "unfree", true );

  if ( unfree )
    {
      if ( ! allows.unfree.value_or( true ) )


        {
          throw PackageCheckFailure(
            nix::fmt( "The package '%s' has an unfree license.\n\n"
                      "Allow unfree packages by setting "
                      "'options.allow.unfree = true' in manifest.toml",
                      packageId ) );
        }


      auto warning = CheckPackageWarning {
        packageId,
        nix::fmt( "The package '%s' has an unfree license, please verify "
                  "the licensing terms of use",
                  packageId ),
      };

      result.emplace_back( warning );
    }

  /**
   * Assume the package is not broken, if field is missing.
   * By default broken packages are denied, so packages without a broken
   * attribute can not be built, without opting to allow broken packages
   * entirely.
   * Additionally, a missing broken attribute, may be the result of not
   * attempting a build at scrape time, thus it's unclear whether the package
   * is in fact broken.
   */
  bool broken = this->info.value( "broken", false );

  if ( broken )
    {
      if ( ! allows.broken.value_or( false ) )
        {
          throw PackageCheckFailure(
            nix::fmt( "The package '%s' is marked as broken.\n\n"
                      "Allow broken packages by setting "
                      "'options.allow.broken = true' in manifest.toml",
                      packageId ) );
        }

      auto warning = CheckPackageWarning {
        packageId,
        nix::fmt( "The package '%s' is marked as broken, it may not behave as "
                  "expected during runtime.",
                  packageId ),
      };

      result.emplace_back( warning );
    }

  // TODO: check more package metadata

  return result;
}

void
to_json( nlohmann::json & jto, const CheckPackageWarning & result )
{
  jto = nlohmann::json {
    { "package", result.packageId },
    { "message", result.message },
  };
}


/* -------------------------------------------------------------------------- */

void
LockfileRaw::clear()
{
  this->manifest.clear();
  this->registry.clear();
  this->packages        = std::unordered_map<System, SystemPackages> {};
  this->lockfileVersion = 0;
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, LockfileRaw & raw )
{
  assertIsJSONObject<InvalidLockfileException>( jfrom, "lockfile" );
  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "manifest" )
        {
          try
            {
              value.get_to( raw.manifest );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidLockfileException( "couldn't parse lockfile field '"
                                                + key + "'",
                                              extract_json_errmsg( err ) );
            }
        }
      else if ( key == "registry" )
        {
          try
            {
              value.get_to( raw.registry );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidLockfileException( "couldn't parse lockfile field '"
                                                + key + "'",
                                              extract_json_errmsg( err ) );
            }
        }
      else if ( key == "packages" )
        {
          if ( ! value.is_object() )
            {
              assertIsJSONObject<InvalidLockfileException>(
                jfrom,
                "lockfile 'packages' field" );
            }
          for ( const auto & [system, descriptors] : value.items() )
            {
              SystemPackages sysPkgs;
              for ( const auto & [pid, descriptor] : descriptors.items() )
                {
                  if ( descriptor.is_null() )
                    {
                      sysPkgs.emplace( pid, std::nullopt );
                      continue;
                    }
                  else
                    {
                      try
                        {
                          sysPkgs.emplace( pid,
                                           descriptor.get<LockedPackageRaw>() );
                        }
                      catch ( nlohmann::json::exception & err )
                        {
                          throw InvalidLockfileException(
                            "couldn't parse lockfile field 'packages." + system
                              + "." + pid + "'",
                            extract_json_errmsg( err ) );
                        }
                    }
                }
              raw.packages.emplace( system, std::move( sysPkgs ) );
            }
        }
      else if ( key == "lockfile-version" )
        {
          try
            {
              value.get_to( raw.lockfileVersion );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidLockfileException( "couldn't parse lockfile field '"
                                                + key + "'",
                                              extract_json_errmsg( err ) );
            }
        }
      else
        {
          throw InvalidLockfileException( "encountered unexpected field '" + key
                                          + "' while parsing locked package" );
        }
    }
}


void
to_json( nlohmann::json & jto, const LockfileRaw & raw )
{
  jto = { { "manifest", raw.manifest },
          { "registry", raw.registry },
          { "packages", raw.packages },
          { "lockfile-version", raw.lockfileVersion } };
}


/* -------------------------------------------------------------------------- */


}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
