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

#include <nix/hash.hh>

#include "flox/core/util.hh"
#include "flox/resolver/lockfile.hh"


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
  for ( const InstallDescriptors & group :
        this->getManifest().getGroupedDescriptors() )
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
                        "invalid group `" + *descriptor->second.group
                        + "' uses multiple inputs" );
                    }
                  else
                    {
                      throw InvalidLockfileException(
                        "invalid toplevel group uses multiple inputs" );
                    }
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
                "manifest `registry.inputs." + name
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

Lockfile::Lockfile( std::filesystem::path lockfilePath )
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
                "couldn't parse locked input field `" + key + "'",
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
                "couldn't parse locked input field `" + key + "'",
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
                "couldn't parse locked input field `" + key + "'",
                extract_json_errmsg( err ) );
            }
        }
      else
        {
          throw InvalidLockfileException( "encountered unexpected field `" + key
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
                "couldn't parse package input field `" + key + "'",
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
                "couldn't parse package input field `" + key + "'",
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
                "couldn't parse package input field `" + key + "'",
                extract_json_errmsg( err ) );
            }
        }
      else if ( key == "info" ) { raw.info = value; }
      else
        {
          throw InvalidLockfileException( "encountered unexpected field `" + key
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
              throw InvalidLockfileException( "couldn't parse lockfile field `"
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
              throw InvalidLockfileException( "couldn't parse lockfile field `"
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
                "lockfile `packages' field" );
            }
          for ( const auto & [system, descriptors] : value.items() )
            {
              SystemPackages sysPkgs;
              for ( const auto & [pid, descriptor] : descriptors.items() )
                {
                  try
                    {
                      sysPkgs.emplace( pid,
                                       descriptor.get<LockedPackageRaw>() );
                    }
                  catch ( nlohmann::json::exception & err )
                    {
                      throw InvalidLockfileException(
                        "couldn't parse lockfile field `packages." + system
                          + "." + pid + "'",
                        extract_json_errmsg( err ) );
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
              throw InvalidLockfileException( "couldn't parse lockfile field `"
                                                + key + "'",
                                              extract_json_errmsg( err ) );
            }
        }
      else
        {
          throw InvalidLockfileException( "encountered unexpected field `" + key
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
          std::remove( this->lockfileRaw.registry.priority.begin(),
                       this->lockfileRaw.registry.priority.end(),
                       elem->first );
          this->lockfileRaw.registry.inputs.erase( elem->first );
          ++count;
        }
      else { ++elem; }
    }

  return count;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
