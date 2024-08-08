/* ========================================================================== *
 *
 * @file resolver/manifest-raw.cc
 *
 * @brief An abstract description of an environment in its unresolved state.
 *        This file contains the implementation of the
 *        @a flox::resolver::ManifestRaw struct, and associated JSON parsers.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <map>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

#include <nix/fetchers.hh>
#include <nix/flake/flakeref.hh>
#include <nix/ref.hh>
#include <nlohmann/json.hpp>

#include "flox/core/types.hh"
#include "flox/core/util.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/registry.hh"
#include "flox/resolver/descriptor.hh"
#include "flox/resolver/manifest-raw.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

void
Options::merge( const Options & overrides )
{
  if ( overrides.systems.has_value() ) { this->systems = overrides.systems; }

  if ( overrides.allow.has_value() )
    {
      if ( ! this->allow.has_value() ) { this->allow = overrides.allow; }
      else
        {
          if ( overrides.allow->unfree.has_value() )
            {
              this->allow->unfree = overrides.allow->unfree;
            }
          if ( overrides.allow->broken.has_value() )
            {
              this->allow->broken = overrides.allow->broken;
            }
          if ( overrides.allow->licenses.has_value() )
            {
              this->allow->licenses = overrides.allow->licenses;
            }
        }
    }

  if ( overrides.semver.has_value() )
    {
      if ( ! this->semver.has_value() ) { this->semver = overrides.semver; }
      else
        {
          if ( overrides.semver->preferPreReleases.has_value() )
            {
              this->semver->preferPreReleases
                = overrides.semver->preferPreReleases;
            }
        }
    }

  if ( overrides.packageGroupingStrategy.has_value() )
    {
      this->packageGroupingStrategy = overrides.packageGroupingStrategy;
    }

  if ( overrides.activationStrategy.has_value() )
    {
      this->activationStrategy = overrides.activationStrategy;
    }
}


/* -------------------------------------------------------------------------- */

Options::operator pkgdb::PkgQueryArgs() const
{
  pkgdb::PkgQueryArgs args;

  if ( this->systems.has_value() ) { args.systems = *this->systems; }

  if ( this->allow.has_value() )
    {
      if ( this->allow->unfree.has_value() )
        {
          args.allowUnfree = *this->allow->unfree;
        }
      if ( this->allow->broken.has_value() )
        {
          args.allowBroken = *this->allow->broken;
        }
      args.licenses = this->allow->licenses;
    }

  if ( this->semver.has_value() && this->semver->preferPreReleases.has_value() )
    {
      args.preferPreReleases = *this->semver->preferPreReleases;
    }

  return args;
}


/* -------------------------------------------------------------------------- */

static void
from_json( const nlohmann::json & jfrom, Options::Semver & semver )
{
  assertIsJSONObject<InvalidManifestFileException>(
    jfrom,
    "manifest field 'options.semver'" );

  /* Clear fields. */
  semver.preferPreReleases = std::nullopt;

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "prefer-pre-releases" || key == "allow-pre-releases" )
        {
          try
            {
              value.get_to( semver.preferPreReleases );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'options.semver."
                "prefer-pre-releases' with value: "
                + value.dump() );
            }
        }
      else
        {
          throw InvalidManifestFileException(
            "unrecognized manifest field 'options.semver." + key + "'." );
        }
    }
}


static void
to_json( nlohmann::json & jto, const Options::Semver & semver )
{
  if ( semver.preferPreReleases.has_value() )
    {
      jto = { { "prefer-pre-releases", *semver.preferPreReleases } };
    }
  else { jto = nlohmann::json::object(); }
}


/* -------------------------------------------------------------------------- */

static void
from_json( const nlohmann::json & jfrom, Options::Allows & allow )
{
  assertIsJSONObject<InvalidManifestFileException>(
    jfrom,
    "manifest field 'options.allow'" );

  /* Clear fields. */
  allow.licenses = std::nullopt;
  allow.unfree   = std::nullopt;
  allow.broken   = std::nullopt;

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "unfree" )
        {
          try
            {
              value.get_to( allow.unfree );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'options.allow.unfree' "
                "with value: "
                + value.dump() );
            }
        }
      else if ( key == "broken" )
        {
          try
            {
              value.get_to( allow.broken );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'options.allow.broken' "
                "with value: "
                + value.dump() );
            }
        }
      else if ( key == "licenses" )
        {
          try
            {
              value.get_to( allow.licenses );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'options.allow.licenses' "
                "with value: "
                + value.dump() );
            }
        }
      else
        {
          throw InvalidManifestFileException(
            "unrecognized manifest field 'options.allow." + key + "'." );
        }
    }
}


static void
to_json( nlohmann::json & jto, const Options::Allows & allow )
{
  if ( allow.unfree.has_value() ) { jto = { { "unfree", *allow.unfree } }; }
  else { jto = nlohmann::json::object(); }

  if ( allow.broken.has_value() ) { jto.emplace( "broken", *allow.broken ); }
  if ( allow.licenses.has_value() )
    {
      jto.emplace( "licenses", *allow.licenses );
    }
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, Options & opts )
{
  assertIsJSONObject<InvalidManifestFileException>(
    jfrom,
    "manifest field 'options'" );

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "systems" )
        {
          try
            {
              value.get_to( opts.systems );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'options.systems' with value: "
                + value.dump() );
            }
        }
      else if ( key == "allow" )
        {
          /* Rely on the underlying exception handlers. */
          value.get_to( opts.allow );
        }
      else if ( key == "semver" )
        {
          /* Rely on the underlying exception handlers. */
          Options::Semver semver;
          value.get_to( semver );
          opts.semver = semver;
        }
      else if ( key == "package-grouping-strategy" )
        {
          try
            {
              value.get_to( opts.packageGroupingStrategy );
            }
          catch ( const nlohmann::json & exception )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field "
                "'options.package-grouping-strategy' with value: "
                + value.dump() );
            }
        }
      else if ( key == "activation-strategy" )
        {
          try
            {
              value.get_to( opts.activationStrategy );
            }
          catch ( const nlohmann::json & exception )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field "
                "'options.activation-strategy' with value: "
                + value.dump() );
            }
        }
      /* Not used within pkgdb */
      else if ( key == "cuda-detection" ) { ; }
      else
        {
          throw InvalidManifestFileException(
            "unrecognized manifest field 'options." + key + "'." );
        }
    }
}


void
to_json( nlohmann::json & jto, const Options & opts )
{
  if ( opts.systems.has_value() ) { jto = { { "systems", *opts.systems } }; }
  else { jto = nlohmann::json::object(); }

  if ( opts.allow.has_value() ) { jto.emplace( "allow", *opts.allow ); }

  if ( opts.semver.has_value() ) { jto.emplace( "semver", *opts.semver ); }

  if ( opts.packageGroupingStrategy.has_value() )
    {
      jto.emplace( "package-grouping-strategy", *opts.packageGroupingStrategy );
    }

  if ( opts.activationStrategy.has_value() )
    {
      jto.emplace( "activation-strategy", *opts.activationStrategy );
    }
}


/* -------------------------------------------------------------------------- */

GlobalManifestRaw::operator GlobalManifestRawGA() const
{
  if ( this->registry.has_value()
       && ( ( *this->registry ) != getGARegistry() ) )
    {
      throw InvalidManifestFileException(
        "global manifest 'registry' does not match the GA registry" );
    }
  return GlobalManifestRawGA( this->options );
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, GlobalManifestRaw & manifest )
{
  assertIsJSONObject<InvalidManifestFileException>( jfrom, "global manifest" );

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "registry" ) { value.get_to( manifest.registry ); }
      else if ( key == "options" ) { value.get_to( manifest.options ); }
      else
        {
          throw InvalidManifestFileException(
            "unrecognized global manifest field: '" + key + "'." );
        }
    }
  manifest.check();
}


void
to_json( nlohmann::json & jto, const GlobalManifestRaw & manifest )
{
  manifest.check();
  jto = nlohmann::json::object();

  if ( manifest.options.has_value() ) { jto["options"] = *manifest.options; }
  if ( manifest.registry.has_value() ) { jto["registry"] = *manifest.registry; }
}


/* -------------------------------------------------------------------------- */

void
EnvBaseRaw::check() const
{
  if ( this->floxhub.has_value() && this->dir.has_value() )
    {
      throw InvalidManifestFileException(
        "manifest may only define one of 'env-base.floxhub' or 'env-base.dir' "
        "fields." );
    }
}


/* -------------------------------------------------------------------------- */

static void
from_json( const nlohmann::json & jfrom, EnvBaseRaw & env )
{
  assertIsJSONObject<InvalidManifestFileException>(
    jfrom,
    "manifest field 'env-base'" );

  /* Clear fields. */
  env.dir     = std::nullopt;
  env.floxhub = std::nullopt;

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "floxhub" )
        {
          try
            {
              value.get_to( env.floxhub );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'env-base.floxhub' with value: "
                + value.dump() );
            }
        }
      else if ( key == "dir" )
        {
          try
            {
              value.get_to( env.dir );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'env-base.dir' with value: "
                + value.dump() );
            }
        }
      else
        {
          throw InvalidManifestFileException(
            "unrecognized manifest field 'env-base." + key + "'." );
        }
    }
  env.check();
}


static void
to_json( nlohmann::json & jto, const EnvBaseRaw & env )
{
  env.check();
  if ( env.dir.has_value() ) { jto = { { "dir", *env.dir } }; }
  else if ( env.floxhub.has_value() ) { jto = { { "floxhub", *env.floxhub } }; }
  else { jto = nlohmann::json::object(); }
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, ProfileScriptsRaw & profile )
{
  assertIsJSONObject<InvalidManifestFileException>(
    jfrom,
    "manifest field 'profile'" );

  /* Clear fields */
  profile.common = std::nullopt;
  profile.bash   = std::nullopt;
  profile.fish   = std::nullopt;
  profile.tcsh   = std::nullopt;
  profile.zsh    = std::nullopt;

  /* Iterate over keys of the JSON object */
  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "common" )
        {
          try
            {
              value.get_to( profile.common );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'profile.common' with value: "
                + value.dump() );
            }
        }
      else if ( key == "bash" )
        {
          try
            {
              value.get_to( profile.bash );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'profile.bash' with value: "
                + value.dump() );
            }
        }
      else if ( key == "fish" )
        {
          try
            {
              value.get_to( profile.fish );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'profile.fish' with value: "
                + value.dump() );
            }
        }
      else if ( key == "tcsh" )
        {
          try
            {
              value.get_to( profile.tcsh );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'profile.tcsh' with value: "
                + value.dump() );
            }
        }
      else if ( key == "zsh" )
        {
          try
            {
              value.get_to( profile.zsh );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'profile.zsh' with value: "
                + value.dump() );
            }
        }
      else
        {
          throw InvalidManifestFileException(
            "unrecognized shell specific profile in manifest 'profile." + key
            + "'." );
        }
    }
}

static void
to_json( nlohmann::json & jto, const ProfileScriptsRaw & profile )
{
  jto = nlohmann::json::object();
  if ( profile.common.has_value() ) { jto["common"] = profile.common.value(); }
  if ( profile.bash.has_value() ) { jto["bash"] = profile.bash.value(); }
  if ( profile.fish.has_value() ) { jto["fish"] = profile.fish.value(); }
  if ( profile.tcsh.has_value() ) { jto["tcsh"] = profile.tcsh.value(); }
  if ( profile.zsh.has_value() ) { jto["zsh"] = profile.zsh.value(); }
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, HookRaw & hook )
{
  assertIsJSONObject<InvalidManifestFileException>( jfrom,
                                                    "manifest field 'hook'" );

  /* Clear fields. */
  hook.script     = std::nullopt;
  hook.onActivate = std::nullopt;

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "script" )
        {
          try
            {
              value.get_to( hook.script );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'hook.script' with value: "
                + value.dump() );
            }
        }
      else if ( key == "on-activate" )
        {
          try
            {
              value.get_to( hook.onActivate );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse manifest field 'hook.on-activate' with value: "
                + value.dump() );
            }
        }
      else
        {
          throw InvalidManifestFileException(
            "unrecognized manifest field 'hook." + key + "'." );
        }
    }

  hook.check();
}


static void
to_json( nlohmann::json & jto, const HookRaw & hook )
{
  hook.check();
  if ( hook.script.has_value() ) { jto = { { "script", *hook.script } }; }
  else if ( hook.onActivate.has_value() )
    {
      jto = { { "on-activate", *hook.onActivate } };
    }
  else { jto = nlohmann::json::object(); }
}


/* -------------------------------------------------------------------------- */

void
HookRaw::check() const
{
  if ( this->script.has_value() && this->onActivate.has_value() )
    {
      throw InvalidManifestFileException(
        "hook may only define one of 'hook.script' or `hook.on-activate` "
        "fields." );
    }
}

/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, BuildDescriptorRaw & build )
{
  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "command" ) { value.get_to( build.command ); }
      else
        {
          throw InvalidManifestFileException(
            "unrecognized manifest field 'build." + key + "'." );
        }
    }
}

void
to_json( nlohmann::json & jto, const BuildDescriptorRaw & build )
{
  jto            = nlohmann::json::object();
  jto["command"] = build.command;
}


/* -------------------------------------------------------------------------- */

static std::unordered_map<std::string, std::optional<ManifestDescriptorRaw>>
installFromJSON( const nlohmann::json & install )
{
  assertIsJSONObject( install, "manifest field 'install'" );
  std::unordered_map<std::string, std::optional<ManifestDescriptorRaw>> result;
  for ( const auto & [name, desc] : install.items() )
    {
      if ( desc.is_null() ) { result.emplace( name, std::nullopt ); }
      else  // TODO: parse CLI strings
        {
          try
            {
              result.emplace( name, desc.get<ManifestDescriptorRaw>() );
            }
          catch ( const nlohmann::json::exception & )
            {
              throw InvalidManifestFileException(
                "failed to parse field 'install." + name
                + "' with value: " + desc.dump() );
            }
        }
    }
  return result;
}


/* -------------------------------------------------------------------------- */

static std::unordered_map<std::string, std::string>
varsFromJSON( const nlohmann::json & jfrom )
{
  assertIsJSONObject<InvalidManifestFileException>( jfrom,
                                                    "manifest field 'vars'" );
  std::unordered_map<std::string, std::string> vars;
  for ( const auto & [key, value] : jfrom.items() )
    {
      std::string val;
      try
        {
          value.get_to( val );
        }
      catch ( const nlohmann::json::exception & err )
        {
          throw InvalidManifestFileException( "failed to parse field 'vars."
                                              + key + "' with value: "
                                              + value.dump() );
        }
      vars.emplace( key, std::move( val ) );
    }
  return vars;
}


/* -------------------------------------------------------------------------- */

ManifestRaw::operator ManifestRawGA() const
{
  ManifestRawGA raw( static_cast<GlobalManifestRawGA>(
    static_cast<GlobalManifestRaw>( *this ) ) );
  raw.install = this->install;
  raw.vars    = this->vars;
  raw.hook    = this->hook;
  return raw;
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, ManifestRaw & manifest )
{
  assertIsJSONObject<InvalidManifestFileException>( jfrom, "manifest" );

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "install" )
        {
          if ( value.is_null() )
            {
              manifest.install = std::nullopt;
              continue;
            }
          manifest.install = installFromJSON( value );
        }
      else if ( key == "registry" ) { value.get_to( manifest.registry ); }
      else if ( key == "vars" )
        {
          if ( value.is_null() )
            {
              manifest.vars = std::nullopt;
              continue;
            }
          manifest.vars = varsFromJSON( value );
        }
      else if ( key == "profile" ) { value.get_to( manifest.profile ); }
      else if ( key == "build" ) { value.get_to( manifest.build ); }
      else if ( key == "hook" ) { value.get_to( manifest.hook ); }
      else if ( key == "options" ) { value.get_to( manifest.options ); }
      else if ( key == "env-base" ) { value.get_to( manifest.envBase ); }
      else
        {
          throw InvalidManifestFileException( "unrecognized manifest field: '"
                                              + key + "'." );
        }
    }
  manifest.check();
}


void
to_json( nlohmann::json & jto, const ManifestRaw & manifest )
{
  manifest.check();
  jto = nlohmann::json::object();

  if ( manifest.envBase.has_value() ) { jto["env-base"] = *manifest.envBase; }

  if ( manifest.options.has_value() ) { jto["options"] = *manifest.options; }

  if ( manifest.install.has_value() ) { jto["install"] = *manifest.install; }

  if ( manifest.registry.has_value() ) { jto["registry"] = *manifest.registry; }

  if ( manifest.vars.has_value() ) { jto["vars"] = *manifest.vars; }

  if ( manifest.profile.has_value() ) { jto["profile"] = *manifest.profile; }

  if ( manifest.build.has_value() ) { jto["build"] = *manifest.build; }

  if ( manifest.hook.has_value() ) { jto["hook"] = *manifest.hook; }
}


/* -------------------------------------------------------------------------- */

void
ManifestRaw::check() const
{
  GlobalManifestRaw::check();
  if ( this->envBase.has_value() ) { this->envBase->check(); }
  if ( this->install.has_value() )
    {
      for ( const auto & [iid, desc] : *this->install )
        {
          if ( desc.has_value() ) { desc->check( iid ); }
        }
    }
  if ( this->hook.has_value() ) { this->hook->check(); }
  if ( this->registry.has_value() )
    {
      for ( const auto & [name, input] : this->registry->inputs )
        {
          if ( input.getFlakeRef()->input.getType() == "indirect" )
            {
              throw InvalidManifestFileException(
                "manifest 'registry.inputs." + name
                + ".from.type' may not be \"indirect\"." );
            }
        }
    }
}


/* -------------------------------------------------------------------------- */

nlohmann::json
ManifestRaw::diff( const ManifestRaw & old ) const
{
  return nlohmann::json::diff( nlohmann::json( *this ), old );
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, GlobalManifestRawGA & manifest )
{
  assertIsJSONObject<InvalidManifestFileException>( jfrom, "global manifest" );

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "options" ) { value.get_to( manifest.options ); }
      else
        {
          throw InvalidManifestFileException(
            "unrecognized global manifest field: '" + key + "'." );
        }
    }
  manifest.check();
}


void
to_json( nlohmann::json & jto, const GlobalManifestRawGA & manifest )
{
  manifest.check();
  jto = nlohmann::json::object();

  if ( manifest.options.has_value() ) { jto["options"] = *manifest.options; }
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, ManifestRawGA & manifest )
{
  assertIsJSONObject<InvalidManifestFileException>( jfrom, "manifest" );

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "install" )
        {
          if ( value.is_null() )
            {
              manifest.install = std::nullopt;
              continue;
            }
          manifest.install = installFromJSON( value );
        }
      else if ( key == "vars" )
        {
          if ( value.is_null() )
            {
              manifest.vars = std::nullopt;
              continue;
            }
          manifest.vars = varsFromJSON( value );
        }
      else if ( key == "hook" ) { value.get_to( manifest.hook ); }
      else if ( key == "options" ) { value.get_to( manifest.options ); }
      else
        {
          throw InvalidManifestFileException( "unrecognized manifest field: '"
                                              + key + "'." );
        }
    }
  manifest.check();
}


void
to_json( nlohmann::json & jto, const ManifestRawGA & manifest )
{
  manifest.check();
  jto = nlohmann::json::object();

  if ( manifest.options.has_value() ) { jto["options"] = *manifest.options; }

  if ( manifest.install.has_value() ) { jto["install"] = *manifest.install; }

  if ( manifest.vars.has_value() ) { jto["vars"] = *manifest.vars; }

  if ( manifest.hook.has_value() ) { jto["hook"] = *manifest.hook; }
}


/* -------------------------------------------------------------------------- */

nlohmann::json
ManifestRawGA::diff( const ManifestRawGA & old ) const
{
  return nlohmann::json::diff( nlohmann::json( *this ), old );
}


/* -------------------------------------------------------------------------- */

void
ManifestRawGA::check() const
{
  GlobalManifestRawGA::check();
  if ( this->install.has_value() )
    {
      for ( const auto & [iid, desc] : *this->install )
        {
          if ( desc.has_value() ) { desc->check( iid ); }
        }
    }
  if ( this->hook.has_value() ) { this->hook->check(); }
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
