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
Lockfile::checkGroups() const
{
  for ( const auto & [_, group] : this->getManifest().getGroupedDescriptors() )
    {
      for ( const auto & system : this->manifest.getSystems() )
        {
          std::optional<LockedInputRaw> groupInput;
          for ( const auto & [iid, descriptor] : group )
            {
              /* Handle system skips.  */
              if ( descriptor.systems.has_value()
                   && ( std::find( this->manifest.getSystems().begin(),
                                   this->manifest.getSystems().end(),
                                   system )
                        == descriptor.systems->end() ) )
                {
                  continue;
                }

              auto maybeSystem = this->lockfileRaw.packages.find( system );
              if ( maybeSystem == this->lockfileRaw.packages.end() )
                {
                  continue;
                }

              auto maybeLocked = maybeSystem->second.at( iid );

              /* Package was unresolved, we don't enforce `optional' here. */
              if ( ! maybeLocked.has_value() ) { continue; }

              if ( ! groupInput.has_value() )
                {
                  groupInput = maybeLocked->input;
                }
              else if ( groupInput->fingerprint
                        != maybeLocked->input.fingerprint )
                {
                  if ( auto descriptor = group.begin();
                       descriptor != group.end()
                       && descriptor->second.group.has_value() )
                    {
                      throw InvalidLockfileException(
                        "invalid group '" + *descriptor->second.group
                        + "' uses multiple inputs" );
                    }

                  throw InvalidLockfileException(
                    "invalid toplevel group uses multiple inputs" );
                }
            }
        }
    }
}


/* -------------------------------------------------------------------------- */

void
Lockfile::check() const
{
  this->lockfileRaw.check();
  if ( this->getManifestRaw().registry.has_value() )
    {
      for ( const auto & [name, input] :
            this->getManifestRaw().registry->inputs )
        {
          if ( input.getFlakeRef()->input.getType() == "indirect" )
            {
              throw InvalidManifestFileException(
                "manifest 'registry.inputs." + name
                + ".from.type' may not be \"indirect\"." );
            }
        }
    }
  // TODO: check `optional' and `system' skips.
  this->checkGroups();
}


/* -------------------------------------------------------------------------- */

void
Lockfile::init()
{
  this->lockfileRaw.check();

  /* Collect inputs from all locked packages into a registry keyed
   * by fingerprints. */
  for ( const auto & [system, sysPkgs] : this->lockfileRaw.packages )
    {
      for ( const auto & [pid, pkg] : sysPkgs )
        {
          if ( ! pkg.has_value() ) { continue; }
          this->packagesRegistryRaw.inputs.try_emplace(
            pkg->input.fingerprint.to_string( nix::Base16, false ),
            static_cast<RegistryInput>( pkg->input ) );
        }
    }

  this->manifest = EnvironmentManifest( this->lockfileRaw.manifest );

  this->check();
}


/* -------------------------------------------------------------------------- */

/** @brief Read a flox::resolver::Lockfile from a file. */
static LockfileRaw
readLockfileFromPath( const std::filesystem::path & lockfilePath )
{
  if ( ! std::filesystem::exists( lockfilePath ) )
    {
      throw InvalidLockfileException( "no such path: "
                                      + lockfilePath.string() );
    }
  return readAndCoerceJSON( lockfilePath );
}

/* -------------------------------------------------------------------------- */

Lockfile::Lockfile( const std::filesystem::path & lockfilePath )
  : lockfileRaw( readLockfileFromPath( lockfilePath ) )
{
  this->init();
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, LockedInputRaw & raw )
{
  assertIsJSONObject<InvalidLockfileException>( jfrom, "locked input" );

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "fingerprint" )
        {
          try
            {
              raw.fingerprint = pkgdb::Fingerprint::parseNonSRIUnprefixed(
                value.get<std::string>(),
                nix::htSHA256 );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidLockfileException(
                "couldn't parse locked input field '" + key + "'",
                extract_json_errmsg( err ) );
            }
          catch ( nix::BadHash & err )
            {
              throw InvalidHashException(
                "failed to parse locked input fingerprint",
                err.what() );
            }
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
  jto = { { "fingerprint", raw.fingerprint.to_string( nix::Base16, false ) },
          { "url", raw.url },
          { "attrs", raw.attrs } };
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
LockfileRaw::load_from_content( const nlohmann::json & jfrom )
{
  unsigned version = jfrom["lockfile-version"];
  debugLog( nix::fmt( "lockfile version %d", version ) );

  switch ( version )
    {
      case 0: *this = jfrom; break;
      case 1: this->from_v1_content( jfrom ); break;
      default:
        throw InvalidLockfileException( "unsupported lockfile version",
                                        "only v0 and v1 are supprted" );
    }
}

static void
lockedPackageFromCatalogDescriptor( const nlohmann::json & jfrom,
                                    LockedPackageRaw &     pkg )
{
  std::string attrPath = jfrom["attr_path"];
  std::string system   = jfrom["system"];

  // This would be more appropriately moved to `evalCacheCursorForInput` where
  // we are introducing the concept of a flake, but that code won't know where
  // the attrPath is coming from to make that detemination.
  pkg.attrPath = splitAttrPath( "legacyPackages." + system + "." + attrPath );
  pkg.priority = jfrom["priority"];
  pkg.info     = jfrom;
  pkg.input    = LockedInputRaw();

  pkg.input.url   = jfrom["locked_url"];
  pkg.input.attrs = nix::fetchers::Attrs();
  // These attributes are needed by the current builder, and not included in the
  // descriptor This will not always be true, but also may not be required to
  // build depending on the path taken for future environment builds.
  if ( std::string supportedUrl = "github:NixOS/nixpkgs";
       pkg.input.url.substr( 0, supportedUrl.size() ) != supportedUrl )
    {
      throw InvalidLockfileException(
        "unsupported lockfile URL for v1 lockfile",
        "must begin with " + supportedUrl );
    }
  pkg.input.attrs["type"]  = "github";
  pkg.input.attrs["owner"] = "NixOS";
  pkg.input.attrs["repo"]  = "nixpkgs";

  std::size_t found = pkg.input.url.rfind( "/" );
  if ( found != std::string::npos )
    {
      pkg.input.attrs["rev"] = pkg.input.url.substr( found + 1 );
    }
}

// void load_optional_string

void
LockfileRaw::from_v1_content( const nlohmann::json & jfrom )
{
  debugLog( nix::fmt( "loading v1 lockfile content" ) );

  unsigned version = jfrom["lockfile-version"];
  if (version != 1)
      throw InvalidLockfileException(
        nix::fmt("trying to parse v%d lockfile as v1", version), "");

  // Set the version
  this->lockfileVersion = version;

  // load vars
  try
    {
      auto value = jfrom["manifest"]["vars"];
      value.get_to( this->manifest.vars );
    }
  catch ( nlohmann::json::exception & err )
    {
      throw InvalidLockfileException(
        "couldn't parse lockfile field 'manifest.vars'",
        extract_json_errmsg( err ) );
    }

  // load hooks
  try
    {
      auto hook = jfrom["manifest"]["hook"];
      hook.get_to(this->manifest.hook);
    }
  catch ( nlohmann::json::exception & err )
    {
      throw InvalidLockfileException(
        "couldn't parse lockfile field 'manifest.hook'",
        extract_json_errmsg( err ) );
    }

  // load profile
  try
    {
      auto hook = jfrom["manifest"]["profile"];
      hook.get_to(this->manifest.profile);
    }
  catch ( nlohmann::json::exception & err )
    {
      throw InvalidLockfileException(
        "couldn't parse lockfile field 'manifest.profile'",
        extract_json_errmsg( err ) );
    }

  // load packages as map<system, map<install-id, package>>
  try
    {
      auto packages = jfrom["packages"];
      for ( const auto & [idx, package] : packages.items() )
        {
          LockedPackageRaw pkg = LockedPackageRaw();
          lockedPackageFromCatalogDescriptor( package, pkg );
          const std::string installId = package["install_id"];
          const std::string system    = package["system"];

          this->packages[system].insert(
            { installId, std::make_optional( pkg ) } );
        }
    }
  catch ( nlohmann::json::exception & err )
    {
      throw InvalidLockfileException( "couldn't parse lockfile field 'groups'",
                                      extract_json_errmsg( err ) );
    }

  debugLog( nix::fmt( "loaded lockfile v1" ) );
}


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

std::size_t
Lockfile::removeUnusedInputs()
{
  /* Check to see if an input was declared in the manifest registry. */
  auto inManifestRegistry = [&]( const std::string & name ) -> bool
  {
    const auto & maybeRegistry = this->getManifestRaw().registry;
    return maybeRegistry.has_value()
           && ( maybeRegistry->inputs.find( name )
                != maybeRegistry->inputs.end() );
  };

  /* Check to see if an input is used by a package. */
  auto inPackagesRegistry = [&]( const std::string & url ) -> bool
  {
    for ( const auto & [name, input] : this->getPackagesRegistryRaw().inputs )
      {
        if ( input.from->to_string() == url ) { return true; }
      }
    return false;
  };

  /* Counts the number of removed inputs. */
  std::size_t count = 0;

  /* Remove. */
  for ( auto elem = this->getRegistryRaw().inputs.begin();
        elem != this->getRegistryRaw().inputs.end(); )
    {
      if ( ( ! inManifestRegistry( elem->first ) )
           && ( ! inPackagesRegistry( elem->second.from->to_string() ) ) )
        {
          lockfileRaw.registry.priority.erase(
            std::remove( this->lockfileRaw.registry.priority.begin(),
                         this->lockfileRaw.registry.priority.end(),
                         elem->first ),
            this->lockfileRaw.registry.priority.end() );
          this->lockfileRaw.registry.inputs.erase( elem->first );
          ++count;
        }
      else { ++elem; }
    }

  return count;
}


/* -------------------------------------------------------------------------- */

std::vector<CheckPackageWarning>
Lockfile::checkPackages( const std::optional<flox::System> & system ) const
{
  std::vector<CheckPackageWarning> warnings;

  auto allows = this->getLockfileRaw()
                  .manifest.options.value_or( Options {} )
                  .allow.value_or( Options::Allows {} );

  for ( auto [system_, packages] : this->getLockfileRaw().packages )
    {
      if ( system.has_value() && system_ != system.value() ) { continue; }

      for ( auto [pid, package] : packages )
        {
          // disabled for current system or optional
          if ( ! package.has_value() ) { continue; }

          auto packageWarnings = package.value().check( pid, allows );
          warnings.insert( warnings.end(),
                           packageWarnings.begin(),
                           packageWarnings.end() );
        }
    }

  return warnings;
}


}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
