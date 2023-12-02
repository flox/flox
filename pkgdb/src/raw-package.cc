/* ========================================================================== *
 *
 * @file raw-package.cc
 *
 * @brief The simplest `Package' implementation comprised of raw values.
 *
 *
 * -------------------------------------------------------------------------- */

#include <string>
#include <string_view>

#include <nlohmann/json.hpp>

#include "flox/core/util.hh"
#include "flox/pkgdb/read.hh"
#include "flox/raw-package.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, RawPackage & pkg )
{
  assertIsJSONObject<flox::pkgdb::PkgDbException>( jfrom, "package" );
  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "path" )
        {
          try
            {
              value.get_to( pkg.path );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `path'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "name" )
        {
          try
            {
              value.get_to( pkg.name );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `name'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "pname" )
        {
          try
            {
              value.get_to( pkg.pname );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `pname'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "version" )
        {
          try
            {
              value.get_to( pkg.version );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `version'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "semver" )
        {
          try
            {
              value.get_to( pkg.semver );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `semver'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "license" )
        {
          try
            {
              value.get_to( pkg.license );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `license'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "outputs" )
        {
          try
            {
              value.get_to( pkg.outputs );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `outputs'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "outputsToInstall" )
        {
          try
            {
              value.get_to( pkg.outputsToInstall );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `outputsToInstall'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "broken" )
        {
          try
            {
              value.get_to( pkg.broken );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `broken'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "unfree" )
        {
          try
            {
              value.get_to( pkg.unfree );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `unfree'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "description" )
        {
          try
            {
              value.get_to( pkg.description );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw flox::pkgdb::PkgDbException(
                "couldn't interpret field `description'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else
        {
          throw flox::pkgdb::PkgDbException( "unrecognized field `" + key
                                             + "'" );
        }
    }
}


/* -------------------------------------------------------------------------- */

void
to_json( nlohmann::json & jto, const flox::RawPackage & pkg )
{
  jto = { { "path", pkg.path },
          {
            "name",
            pkg.name,
          },
          { "pname", pkg.pname },
          { "version", pkg.version },
          { "semver", pkg.semver },
          { "license", pkg.license },
          { "outputs", pkg.outputs },
          { "outputsToInstall", pkg.outputsToInstall },
          { "broken", pkg.broken },
          { "unfree", pkg.unfree },
          { "description", pkg.description } };
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
