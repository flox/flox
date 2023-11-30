/* ========================================================================== *
 *
 * @file resolver/descriptor.cc
 *
 * @brief A set of user inputs used to set input preferences and query
 *        parameters during resolution.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <map>
#include <nlohmann/json.hpp>
#include <optional>
#include <regex>
#include <stddef.h>
#include <string>
#include <string_view>
#include <utility>
#include <variant>
#include <vector>

#include <nix/attrs.hh>
#include <nix/flake/flakeref.hh>

#include "flox/core/types.hh"
#include "flox/core/util.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/resolver/descriptor.hh"
#include "versions.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

static AttrPathGlob
maybeSplitAttrPathGlob( const ManifestDescriptorRaw::AbsPath & absPath );


/* -------------------------------------------------------------------------- */

void
ManifestDescriptorRaw::clear()
{
  this->name              = std::nullopt;
  this->version           = std::nullopt;
  this->path              = std::nullopt;
  this->absPath           = std::nullopt;
  this->systems           = std::nullopt;
  this->optional          = std::nullopt;
  this->packageGroup      = std::nullopt;
  this->packageRepository = std::nullopt;
  this->priority          = std::nullopt;
}


/* -------------------------------------------------------------------------- */

void
ManifestDescriptorRaw::check( std::string iid ) const
{
  if ( this->absPath.has_value() )
    {
      AttrPathGlob glob = maybeSplitAttrPathGlob( *this->absPath );
      if ( glob.size() < 3 )
        {
          throw InvalidManifestDescriptorException(
            "`install." + iid + ".abspath' must have at least three parts." );
        }
      if ( ! glob.at( 0 ).has_value()
           || ( std::find( getDefaultSubtrees().begin(),
                           getDefaultSubtrees().end(),
                           *glob.at( 0 ) )
                == getDefaultSubtrees().end() ) )
        {
          throw InvalidManifestDescriptorException(
            "`install." + iid
            + ".abspath' must have a subtree as its first element" );
        }

      if ( this->path.has_value() )
        {
          for ( auto part = glob.begin() + 2; part != glob.end(); part++ )
            {
              if ( ! part->has_value() )
                {
                  throw InvalidManifestDescriptorException(
                    "`install." + iid
                    + ".abspath' may only have a glob as its "
                      "second element" );
                }
            }
          throw InvalidManifestDescriptorException(
            "`install." + iid + ".path' conflicts with `install.*.abspath'" );
        }

      if ( this->systems.has_value() && glob.at( 1 ).has_value() )
        {
          if ( std::find( this->systems->begin(),
                          this->systems->end(),
                          *glob.at( 1 ) )
               == this->systems->end() )
            {
              throw InvalidManifestDescriptorException(
                "`install." + iid
                + ".systems' list conflicts with `install.*.abspath' "
                  "system specification" );
            }
        }
    }
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Sets either `version` or `semver` on
 *        a `flox::resolver::ManifestDescriptor`.
 *
 * Distinguishes between semver ranges and exact version matchers.
 * @param desc The descriptor to initialize.
 * @param version The version description to parse.
 */
static void
initManifestDescriptorVersion( ManifestDescriptor & desc,
                               const std::string &  version )
{
  /* Strip leading/trailing whitespace. */
  std::string trimmed = trim_copy( version );

  /* Empty is recognized as a glob/_any_ range. */
  if ( trimmed.empty() )
    {
      desc.semver = std::move( trimmed );
      return;
    }

  /* Try a quick detection based on first character.
   * We identify `=` as an explicit _exact version_ match. */
  switch ( trimmed.at( 0 ) )
    {
      case '=': desc.version = trimmed.substr( 1 ); break;

      case '*':
      case '~':
      case '^':
      case '>':
      case '<': desc.semver = std::move( trimmed ); break;

      default:
        /* If it's a valid semver or a date then it's not a range. */
        if ( versions::isSemver( trimmed ) || versions::isDate( trimmed )
             || ( ! versions::isSemverRange( trimmed ) ) )
          {
            desc.version = std::move( trimmed );
          }
        else /* Otherwise, assume a range. */
          {
            desc.semver = std::move( trimmed );
          }
        break;
    }
}


/* -------------------------------------------------------------------------- */

/** @brief Get a `flox::resolver::AttrPathGlob` from a string if necessary. */
static AttrPathGlob
maybeSplitAttrPathGlob( const ManifestDescriptorRaw::AbsPath & absPath )
{
  if ( std::holds_alternative<AttrPathGlob>( absPath ) )
    {
      return std::get<AttrPathGlob>( absPath );
    }
  AttrPathGlob   glob;
  flox::AttrPath path = splitAttrPath( std::get<std::string>( absPath ) );
  size_t         idx  = 0;
  for ( const auto & part : path )
    {
      /* Treat `null' or `*' as a glob. */
      /* TODO we verify that only the second option is a glob elsewhere, but we
       * could do that here instead */
      if ( ( ( part == "null" ) || ( part == "*" ) ) )
        {
          glob.emplace_back( std::nullopt );
        }
      else { glob.emplace_back( part ); }
      ++idx;
    }
  return glob;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Sets various fields on a `flox::resolver::ManifestDescriptor`
 *        based on the `absPath` field.
 * @param desc The descriptor to initialize.
 * @param raw The raw description to parse.
 */
static void
initManifestDescriptorAbsPath( ManifestDescriptor &          desc,
                               const ManifestDescriptorRaw & raw )
{
  if ( ! raw.absPath.has_value() )
    {
      throw InvalidManifestDescriptorException(
        "`abspath' must be set when calling "
        "`flox::resolver::ManifestDescriptor::initManifestDescriptorAbsPath'" );
    }

  /* You might need to parse a globbed attr path, so handle that first. */
  AttrPathGlob glob = maybeSplitAttrPathGlob( *raw.absPath );

  if ( glob.size() < 3 )
    {
      throw InvalidManifestDescriptorException(
        "`abspath' must have at least three parts" );
    }

  const auto & first = glob.front();
  if ( ! first.has_value() )
    {
      throw InvalidManifestDescriptorException(
        "`abspath' may only have a glob as its second element" );
    }
  desc.subtree = Subtree( *first );

  desc.path = AttrPath {};
  for ( auto itr = glob.begin() + 2; itr != glob.end(); ++itr )
    {
      const auto & elem = *itr;
      if ( ! elem.has_value() )
        {
          throw InvalidManifestDescriptorException(
            "`abspath' may only have a glob as its second element" );
        }
      desc.path = AttrPath {};
      for ( auto itr = glob.begin() + 3; itr != glob.end(); ++itr )
        {
          const auto & elem = *itr;
          if ( ! elem.has_value() )
            {
              throw InvalidManifestDescriptorException(
                "`abspath' may only have a glob as its second element" );
            }
          desc.path->emplace_back( *elem );
        }
    }

  desc.path = AttrPath {};
  for ( auto itr = glob.begin() + 2; itr != glob.end(); ++itr )
    {
      const auto & elem = *itr;
      if ( ! elem.has_value() )
        {
          throw InvalidManifestDescriptorException(
            "`abspath' may only have a glob as its second element" );
        }
      desc.path->emplace_back( *elem );
    }

  const auto & second = glob.at( 1 );
  if ( second.has_value() && ( ( *second ) != "null" )
       && ( ( *second ) != "*" ) )
    {
      desc.systems = std::vector<std::string> { *second };
      if ( raw.systems.has_value() && ( *raw.systems != *desc.systems ) )
        {
          throw InvalidManifestDescriptorException(
            "`systems' list conflicts with `abspath' system specification" );
        }
    }
}

/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, ManifestDescriptorRaw & descriptor )
{
  if ( ! jfrom.is_object() )
    {
      std::string aOrAn = jfrom.is_array() ? " an " : " a ";
      throw ParseManifestDescriptorRawException(
        "manifest descriptor must be an object, but is" + aOrAn
        + std::string( jfrom.type_name() ) + '.' );
    }

  /* Clear fields. */
  descriptor.clear();

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "name" )
        {
          try
            {
              value.get_to( descriptor.name );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseManifestDescriptorRawException(
                "couldn't interpret field `name'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "version" )
        {
          try
            {
              value.get_to( descriptor.version );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseManifestDescriptorRawException(
                "couldn't interpret field `version'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "path" )
        {
          try
            {
              value.get_to( descriptor.path );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseManifestDescriptorRawException(
                "couldn't interpret field `path'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "abspath" )
        {
          try
            {
              value.get_to( descriptor.absPath );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseManifestDescriptorRawException(
                "couldn't interpret field `abspath'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "systems" )
        {
          try
            {
              value.get_to( descriptor.systems );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseManifestDescriptorRawException(
                "couldn't interpret field `systems'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "optional" )
        {
          try
            {
              value.get_to( descriptor.optional );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseManifestDescriptorRawException(
                "couldn't interpret field `optional'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "package-group" )
        {
          try
            {
              value.get_to( descriptor.packageGroup );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseManifestDescriptorRawException(
                "couldn't interpret field `package-group'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "package-repository" )
        {
          try
            {
              value.get_to( descriptor.packageRepository );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseManifestDescriptorRawException(
                "couldn't interpret field `package-repository'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else if ( key == "priority" )
        {
          try
            {
              value.get_to( descriptor.priority );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseManifestDescriptorRawException(
                "couldn't interpret field `priority'",
                flox::extract_json_errmsg( e ) );
            }
        }
      else
        {
          throw ParseManifestDescriptorRawException(
            "encountered unrecognized field `" + key
            + "' while parsing manifest descriptor" );
        }
    }
}

void
to_json( nlohmann::json & jto, const ManifestDescriptorRaw & descriptor )
{
  if ( descriptor.name.has_value() ) { jto["name"] = *descriptor.name; }
  if ( descriptor.version.has_value() )
    {
      jto["version"] = *descriptor.version;
    }
  if ( descriptor.path.has_value() ) { jto["path"] = *descriptor.path; }
  if ( descriptor.absPath.has_value() )
    {
      jto["abspath"] = *descriptor.absPath;
    }
  if ( descriptor.systems.has_value() )
    {
      jto["systems"] = *descriptor.systems;
    }
  if ( descriptor.optional.has_value() )
    {
      jto["optional"] = *descriptor.optional;
    }
  if ( descriptor.packageGroup.has_value() )
    {
      jto["package-group"] = *descriptor.packageGroup;
    }
  if ( descriptor.packageRepository.has_value() )
    {
      jto["package-repository"] = *descriptor.packageRepository;
    }
  if ( descriptor.priority.has_value() )
    {
      jto["priority"] = *descriptor.priority;
    }
}


/* -------------------------------------------------------------------------- */

ManifestDescriptor::ManifestDescriptor( const ManifestDescriptorRaw & raw )
  : name( raw.name )
  , optional( raw.optional.value_or( false ) )
  , group( raw.packageGroup )
{
  /* Determine if `version' was a range or not.
   * NOTE: The string "4.2.0" is not a range, but "4.2" is!
   *       If you want to explicitly match the `version` field with "4.2" then
   *       you need to use "=4.2". */
  if ( raw.version.has_value() )
    {
      initManifestDescriptorVersion( *this, *raw.version );
    }

  /* You have to split `abspath' before doing most other fields. */
  if ( raw.absPath.has_value() )
    {
      initManifestDescriptorAbsPath( *this, raw );
    }

  /* Only set if it wasn't handled by `absPath`. */
  if ( ( ! this->systems.has_value() ) && raw.systems.has_value() )
    {
      this->systems = *raw.systems;
    }

  if ( raw.path.has_value() )
    {
      /* Split relative path */
      flox::AttrPath path;
      if ( std::holds_alternative<std::string>( *raw.path ) )
        {
          path = splitAttrPath( std::get<std::string>( *raw.path ) );
        }
      else { path = std::get<AttrPath>( *raw.path ); }

      if ( this->path.has_value() )
        {
          if ( this->path != path )
            {
              throw InvalidManifestDescriptorException(
                "`path' conflicts with with `abspath'" );
            }
        }
      else { this->path = path; }
    }

  if ( raw.packageRepository.has_value() )
    {
      if ( std::holds_alternative<std::string>( *raw.packageRepository ) )
        {
          this->input
            = parseFlakeRef( std::get<std::string>( *raw.packageRepository ) );
        }
      else
        {
          this->input = nix::FlakeRef::fromAttrs(
            std::get<nix::fetchers::Attrs>( *raw.packageRepository ) );
        }
    }

  if ( raw.priority.has_value() ) { this->priority = *raw.priority; }
}


/* -------------------------------------------------------------------------- */

void
ManifestDescriptor::clear()
{
  this->name     = std::nullopt;
  this->optional = false;
  this->group    = std::nullopt;
  this->version  = std::nullopt;
  this->semver   = std::nullopt;
  this->subtree  = std::nullopt;
  this->systems  = std::nullopt;
  this->path     = std::nullopt;
  this->input    = std::nullopt;
  this->priority = 5;
}


/* -------------------------------------------------------------------------- */

pkgdb::PkgQueryArgs &
ManifestDescriptor::fillPkgQueryArgs( pkgdb::PkgQueryArgs & pqa ) const
{
  /* Must exactly match either `pname' or `attrName'. */
  if ( this->name.has_value() ) { pqa.pnameOrAttrName = *this->name; }

  if ( this->version.has_value() ) { pqa.version = *this->version; }
  else if ( this->semver.has_value() )
    {
      pqa.semver = *this->semver;
      /* Use `preferPreRelease' on `~<VERSION>-<TAG>' ranges. */
      if ( this->semver->at( 0 ) == '~' )
        {
          pqa.preferPreReleases = std::regex_match(
            *this->semver,
            std::regex( "~[^ ]+-.*", std::regex::ECMAScript ) );
        }
    }

  if ( this->subtree.has_value() )
    {
      pqa.subtrees = std::vector<Subtree> { *this->subtree };
    }

  if ( this->systems.has_value() ) { pqa.systems = *this->systems; }

  pqa.relPath = this->path;

  return pqa;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
