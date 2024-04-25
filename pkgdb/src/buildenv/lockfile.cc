//  To parse this JSON data, first install
//
//      json.hpp  https://github.com/nlohmann/json
//
//  Then include this file, and then do
//
//     Lockfile data = nlohmann::json::parse(jsonString);

#pragma once

#include "flox/core/util.hh"
#include <optional>


// #ifndef NLOHMANN_OPT_HELPER
// #  define NLOHMANN_OPT_HELPER
// namespace nlohmann {
// template<typename T>
// struct adl_serializer<std::shared_ptr<T>>
// {
//   static void
//   to_json( nlohmann::json & jto, const std::shared_ptr<T> & opt )
//   {
//     if ( ! opt ) { j = nullptr; }
//     else { j = *opt; }
//   }

//   static std::shared_ptr<T>
//   from_json( const nlohmann::json & jfrom )
//   {
//     if ( j.is_null() ) { return std::make_shared<T>(); }
//     else { return std::make_shared<T>( j.get<T>() ); }
//   }
// };
// template<typename T>
// struct adl_serializer<std::optional<T>>
// {
//   static void
//   to_json( nlohmann::json & jto, const std::optional<T> & opt )
//   {
//     if ( ! opt ) { j = nullptr; }
//     else { j = *opt; }
//   }

//   static std::optional<T>
//   from_json( const nlohmann::json & jfrom )
//   {
//     if ( j.is_null() ) { return std::make_optional<T>(); }
//     else { return std::make_optional<T>( j.get<T>() ); }
//   }
// };
// }  // namespace nlohmann
// #endif

namespace pkgdb { namespace buildenv {
using nlohmann::json;

#ifndef NLOHMANN_UNTYPED_pkgdb_buildenv_HELPER
#  define NLOHMANN_UNTYPED_pkgdb_buildenv_HELPER
inline json
get_untyped( const nlohmann::json & jto, const char * property )
{
  if ( j.find( property ) != j.end() ) { return j.at( property ).get<json>(); }
  return json();
}

inline json
get_untyped( const nlohmann::json & jto, std::string property )
{
  return get_untyped( j, property.data() );
}
#endif

#ifndef NLOHMANN_OPTIONAL_pkgdb_buildenv_HELPER
#  define NLOHMANN_OPTIONAL_pkgdb_buildenv_HELPER
template<typename T>
inline std::shared_ptr<T>
get_heap_optional( const nlohmann::json & jto, const char * property )
{
  auto it = j.find( property );
  if ( it != j.end() && ! it->is_null() )
    {
      return j.at( property ).get<std::shared_ptr<T>>();
    }
  return std::shared_ptr<T>();
}

template<typename T>
inline std::shared_ptr<T>
get_heap_optional( const nlohmann::json & jto, std::string property )
{
  return get_heap_optional<T>( j, property.data() );
}
template<typename T>
inline std::optional<T>
get_stack_optional( const nlohmann::json & jto, const char * property )
{
  auto it = j.find( property );
  if ( it != j.end() && ! it->is_null() )
    {
      return j.at( property ).get<std::optional<T>>();
    }
  return std::optional<T>();
}

template<typename T>
inline std::optional<T>
get_stack_optional( const nlohmann::json & jto, std::string property )
{
  return get_stack_optional<T>( j, property.data() );
}
#endif

/**
 * Hooks that are run at various times during the lifecycle of the manifest in a
 * known shell environment.
 */
struct ManifestHook
{
  /**
   * A script that is run at activation time, in a flox provided bash shell
   */
  std::optional<std::string> on_activate;
};

struct ManifestPackageDescriptor
{
  std::optional<bool>                     optional;
  std::optional<std::string>              package_group;
  std::string                             pkg_path;
  std::optional<int64_t>                  priority;
  std::optional<std::vector<std::string>> systems;
  std::optional<std::string>              version;
};

/**
 * Options that control what types of packages are allowed.
 */
struct Allows
{
  /**
   * Whether to allow packages that are marked as `broken`
   */
  std::optional<bool> broken;
  /**
   * A list of license descriptors that are allowed
   */
  std::optional<std::vector<std::string>> licenses;
  /**
   * Whether to allow packages that are marked as `unfree`
   */
  std::optional<bool> unfree;
};

/**
 * Options that control how semver versions are resolved.
 */
struct SemverOptions
{
  /**
   * Whether to prefer pre-release versions when resolving
   */
  std::optional<bool> prefer_pre_releases;
};

/**
 * Options that control the behavior of the manifest.
 */
struct ManifestOptions
{
  /**
   * Options that control what types of packages are allowed.
   */
  std::optional<Allows> allows;
  /**
   * Options that control how semver versions are resolved.
   */
  std::optional<SemverOptions> semver;
  /**
   * A list of systems that each package is resolved for.
   */
  std::optional<std::vector<std::string>> systems;
};

/**
 * Profile scripts that are run in the user's shell upon activation.
 */
struct ManifestProfile
{
  /**
   * When defined, this hook is run upon activation in a bash shell
   */
  std::optional<std::string> bash;
  /**
   * When defined, this hook is run by _all_ shells upon activation
   */
  std::optional<std::string> common;
  /**
   * When defined, this hook is run upon activation in a fish shell
   */
  std::optional<std::string> fish;
  /**
   * When defined, this hook is run upon activation in a zsh shell
   */
  std::optional<std::string> zsh;
};

/**
 * original manifest that was locked
 *
 * Not meant for writing manifest files, only for reading them. Modifications
 * should be made using the the raw functions in this module.
 */
struct TypedManifestCatalog
{
  /**
   * Hooks that are run at various times during the lifecycle of the manifest in
   * a known shell environment.
   */
  std::optional<ManifestHook> hook;
  /**
   * The packages to install in the form of a map from package name to package
   * descriptor.
   */
  std::optional<std::map<std::string, ManifestPackageDescriptor>> install;
  /**
   * Options that control the behavior of the manifest.
   */
  std::optional<ManifestOptions> options;
  /**
   * Profile scripts that are run in the user's shell upon activation.
   */
  std::optional<ManifestProfile> profile;
  /**
   * Variables that are exported to the shell environment upon activation.
   */
  std::optional<std::map<std::string, std::string>> vars;
  nlohmann::json                                    version;
};

struct LockedPackageCatalog
{
  std::string                        attr_path;
  bool                               broken;
  std::string                        derivation;
  std::string                        description;
  std::string                        license;
  std::string                        locked_url;
  std::string                        name;
  std::map<std::string, std::string> outputs;
  std::vector<std::string>           outputs_to_install;
  std::string                        pname;
  std::string                        rev;
  int64_t                            rev_count;
  std::string                        rev_date;
  std::string                        scrape_date;
  std::vector<std::string>           stabilities;
  std::string                        system;
  bool                               unfree;
  std::string                        version;
};

struct Lockfile
{
  nlohmann::json lockfile_version;
  /**
   * original manifest that was locked
   */
  TypedManifestCatalog manifest;
  /**
   * locked pacakges
   */
  std::vector<LockedPackageCatalog> packages;
};
}}  // namespace pkgdb::buildenv

namespace pkgdb { namespace buildenv {
void
from_json( const nlohmann::json & jfrom, ManifestHook & x );
void
to_json( nlohmann::json & jto, const ManifestHook & x );

void
from_json( const nlohmann::json & jfrom, ManifestPackageDescriptor & x );
void
to_json( nlohmann::json & jto, const ManifestPackageDescriptor & x );

void
from_json( const nlohmann::json & jfrom, Allows & x );
void
to_json( nlohmann::json & jto, const Allows & x );

void
from_json( const nlohmann::json & jfrom, SemverOptions & x );
void
to_json( nlohmann::json & jto, const SemverOptions & x );

void
from_json( const nlohmann::json & jfrom, ManifestOptions & x );
void
to_json( nlohmann::json & jto, const ManifestOptions & x );

void
from_json( const nlohmann::json & jfrom, ManifestProfile & x );
void
to_json( nlohmann::json & jto, const ManifestProfile & x );

void
from_json( const nlohmann::json & jfrom, TypedManifestCatalog & x );
void
to_json( nlohmann::json & jto, const TypedManifestCatalog & x );

void
from_json( const nlohmann::json & jfrom, LockedPackageCatalog & x );
void
to_json( nlohmann::json & jto, const LockedPackageCatalog & x );

void
from_json( const nlohmann::json & jfrom, Lockfile & x );
void
to_json( nlohmann::json & jto, const Lockfile & x );

inline void
from_json( const nlohmann::json & jfrom, ManifestHook & x )
{
  x.on_activate = get_stack_optional<std::string>( j, "on-activate" );
}

inline void
to_json( nlohmann::json & jto, const ManifestHook & x )
{
  j                = json::object();
  j["on-activate"] = x.on_activate;
}

inline void
from_json( const nlohmann::json & jfrom, ManifestPackageDescriptor & x )
{
  x.optional      = get_stack_optional<bool>( j, "optional" );
  x.package_group = get_stack_optional<std::string>( j, "package-group" );
  x.pkg_path      = j.at( "pkg-path" ).get<std::string>();
  x.priority      = get_stack_optional<int64_t>( j, "priority" );
  x.systems = get_stack_optional<std::vector<std::string>>( j, "systems" );
  x.version = get_stack_optional<std::string>( j, "version" );
}

inline void
to_json( nlohmann::json & jto, const ManifestPackageDescriptor & x )
{
  jto = { { "optional", x.optional }, { "package-group", x.package_group },
          { "pkg-path", x.pkg_path }, { "priority", x.priority },
          { "systems", x.systems },   { "version", x.version } };
}

inline void
from_json( const nlohmann::json & jfrom, Allows & x )
{
  x.broken   = get_stack_optional<bool>( j, "broken" );
  x.licenses = get_stack_optional<std::vector<std::string>>( j, "licenses" );
  x.unfree   = get_stack_optional<bool>( j, "unfree" );
}

inline void
to_json( nlohmann::json & jto, const Allows & x )
{
  jto = { { "broken", x.broken },
          { "licenses", x.licenses },
          { "unfree", x.unfree } };
}

inline void
from_json( const nlohmann::json & jfrom, SemverOptions & x )
{
  x.prefer_pre_releases = get_stack_optional<bool>( j, "prefer_pre_releases" );
}

inline void
to_json( nlohmann::json & jto, const SemverOptions & x )
{
  jto = { { "prefer_pre_releases", x.prefer_pre_releases } };
}

inline void
from_json( const nlohmann::json & jfrom, ManifestOptions & x )
{
  x.allows  = get_stack_optional<Allows>( j, "allows" );
  x.semver  = get_stack_optional<SemverOptions>( j, "semver" );
  x.systems = get_stack_optional<std::vector<std::string>>( j, "systems" );
}

inline void
to_json( nlohmann::json & jto, const ManifestOptions & x )
{
  jto = { { "allows", x.allows },
          { "semver", x.semver },
          { "systems", x.systems } };
}

inline void
from_json( const nlohmann::json & jfrom, ManifestProfile & x )
{
  x.bash   = get_stack_optional<std::string>( j, "bash" );
  x.common = get_stack_optional<std::string>( j, "common" );
  x.fish   = get_stack_optional<std::string>( j, "fish" );
  x.zsh    = get_stack_optional<std::string>( j, "zsh" );
}

void
to_json( nlohmann::json & jto, const ManifestProfile & x )
{
  jto = { { "bash", x.bash },
          { "common", x.common },
          { "fish", x.fish },
          { "zsh", x.zsh } };
}

inline void
from_json( const nlohmann::json & jfrom, TypedManifestCatalog & x )
{
  x.hook = get_stack_optional<ManifestHook>( j, "hook" );
  x.install
    = get_stack_optional<std::map<std::string, ManifestPackageDescriptor>>(
      j,
      "install" );
  x.options = get_stack_optional<ManifestOptions>( j, "options" );
  x.profile = get_stack_optional<ManifestProfile>( j, "profile" );
  x.vars    = jfrom.at( "vars" ).get<std::optional<std::map<std::string, std::string>>>();

  get_stack_optional<std::map<std::string, std::string>>( j, "vars" );
  x.version = get_untyped( j, "version" );
}

inline void
to_json( nlohmann::json & jto, const TypedManifestCatalog & x )
{
  jto = { { "hook", x.hook },       { "install", x.install },
          { "options", x.options }, { "profile", x.profile },
          { "vars", x.vars },       { "version", x.version } };
}

inline void
from_json( const nlohmann::json & jfrom, LockedPackageCatalog & x )
{
  x.attr_path   = jfrom.at( "attr_path" ).get<std::string>();
  x.broken      = jfrom.at( "broken" ).get<bool>();
  x.derivation  = jfrom.at( "derivation" ).get<std::string>();
  x.description = jfrom.at( "description" ).get<std::string>();
  x.license     = jfrom.at( "license" ).get<std::string>();
  x.locked_url  = jfrom.at( "locked_url" ).get<std::string>();
  x.name        = jfrom.at( "name" ).get<std::string>();
  x.outputs     = jfrom.at( "outputs" ).get<std::map<std::string, std::string>>();
  x.outputs_to_install
    = jfrom.at( "outputs_to_install" ).get<std::vector<std::string>>();
  x.pname       = jfrom.at( "pname" ).get<std::string>();
  x.rev         = jfrom.at( "rev" ).get<std::string>();
  x.rev_count   = jfrom.at( "rev_count" ).get<int64_t>();
  x.rev_date    = jfrom.at( "rev_date" ).get<std::string>();
  x.scrape_date = jfrom.at( "scrape_date" ).get<std::string>();
  x.stabilities = jfrom.at( "stabilities" ).get<std::vector<std::string>>();
  x.system      = jfrom.at( "system" ).get<std::string>();
  x.unfree      = jfrom.at( "unfree" ).get<bool>();
  x.version     = jfrom.at( "version" ).get<std::string>();
}

inline void
to_json( nlohmann::json & jto, const LockedPackageCatalog & x )
{
  jto = { { "attr_path", x.attr_path },
          { "broken", x.broken },
          { "derivation", x.derivation },
          { "description", x.description },
          { "license", x.license },
          { "locked_url", x.locked_url },
          { "name", x.name },
          { "outputs", x.outputs },
          { "outputs_to_install", x.outputs_to_install },
          { "pname", x.pname },
          { "rev", x.rev },
          { "rev_count", x.rev_count },
          { "rev_date", x.rev_date },
          { "scrape_date", x.scrape_date },
          { "stabilities", x.stabilities },
          { "system", x.system },
          { "unfree", x.unfree },
          { "version", x.version } };
}

inline void
from_json( const nlohmann::json & jfrom, Lockfile & x )
{
  x.lockfile_version = get_untyped( jfrom, "lockfile-version" );
  x.manifest         = jfrom.at( "manifest" ).get<TypedManifestCatalog>();
  x.packages = jfrom.at( "packages" ).get<std::vector<LockedPackageCatalog>>();
}

inline void
to_json( nlohmann::json & jto, const Lockfile & x )
{

  jto = { { "lockfile-version", x.lockfile_version },
          { "manifest", x.manifest },
          { "packages", x.packages } };
}
}}  // namespace pkgdb::buildenv
