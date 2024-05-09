/* ========================================================================== *
 *
 * @file resolver/command.cc
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nlohmann/json.hpp>

#include "flox/resolver/command.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

/* Manifest Subcommand */

LockCommand::LockCommand() : parser( "lock" )
{
  this->parser.add_description( "Lock a manifest" );
  this->addGlobalManifestFileOption( this->parser );
  this->addLockfileOption( this->parser );
  this->addGARegistryOption( this->parser );
  /* TODO: make manifest file optional and support locking global manifest. */
  this->addManifestFileArg( this->parser, true );
}


/* -------------------------------------------------------------------------- */

int
LockCommand::run()
{
  // TODO: `RegistryRaw' should drop empty fields.
  nlohmann::json lockfile
    = this->getEnvironment().createLockfile().getLockfileRaw();
  /* Print that bad boii */
  std::cout << lockfile.dump() << '\n';
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

DiffCommand::DiffCommand() : parser( "diff" )
{
  this->parser.add_description( "Diff two manifest files" );

  this->parser.add_argument( "old-manifest" )
    .help( "path to old manifest file" )
    .required()
    .metavar( "OLD-MANIFEST" )
    .action( [&]( const std::string & path )
             { this->oldManifestPath = path; } );

  this->parser.add_argument( "new-manifest" )
    .help( "path to new manifest file" )
    .required()
    .metavar( "NEW-MANIFEST" )
    .action( [&]( const std::string & path ) { this->manifestPath = path; } );
}


/* -------------------------------------------------------------------------- */

const ManifestRaw &
DiffCommand::getManifestRaw()
{
  if ( ! this->manifestRaw.has_value() )
    {
      if ( ! this->manifestPath.has_value() )
        {
          throw FloxException( "you must provide a path to a manifest file." );
        }
      if ( ! std::filesystem::exists( *this->manifestPath ) )
        {
          throw InvalidManifestFileException( "manifest file '"
                                              + this->manifestPath->string()
                                              + "'does not exist." );
        }
      this->manifestRaw = readAndCoerceJSON( *this->manifestPath );
    }
  return *this->manifestRaw;
}


/* -------------------------------------------------------------------------- */

const ManifestRaw &
DiffCommand::getOldManifestRaw()
{
  if ( ! this->oldManifestRaw.has_value() )
    {
      if ( ! this->oldManifestPath.has_value() )
        {
          throw FloxException(
            "you must provide a path to an old manifest file." );
        }
      if ( ! std::filesystem::exists( *this->oldManifestPath ) )
        {
          throw InvalidManifestFileException( "old manifest file '"
                                              + this->oldManifestPath->string()
                                              + "'does not exist." );
        }
      this->oldManifestRaw = readAndCoerceJSON( *this->oldManifestPath );
    }
  return *this->oldManifestRaw;
}


/* -------------------------------------------------------------------------- */

int
DiffCommand::run()
{
  auto diff = this->getOldManifestRaw().diff( this->getManifestRaw() );
  std::cout << diff.dump() << '\n';
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

UpdateCommand::UpdateCommand() : parser( "update" )
{
  this->parser.add_description( "Update environment inputs" );
  this->addGlobalManifestFileOption( this->parser );
  this->addLockfileOption( this->parser );
  this->addGARegistryOption( this->parser );

  this->addManifestFileArg( this->parser, false );

  this->parser.add_argument( "inputs" )
    .help( "names of inputs to update" )
    .metavar( "INPUTS..." )
    .remaining()
    .action(
      [&]( const std::string & inputName )
      {
        if ( ! this->inputNames.has_value() )
          {
            this->inputNames = std::vector<std::string>();
          }
        this->inputNames->emplace_back( inputName );
      } );
}


/* -------------------------------------------------------------------------- */

int
UpdateCommand::run()
{
  /* If the manifest doesn't have a value, assume we're updating the global
   * manifest, and set a dummy empty manifest.
   * TODO: be less hacky. */
  if ( ! this->getManifestRaw().has_value() )
    {
      this->setManifestRaw( ManifestRaw {} );
    }
  nlohmann::json lockfile;
  if ( auto maybeLockfile = this->getLockfile(); maybeLockfile.has_value() )
    {
      auto lockedRaw         = maybeLockfile->getLockfileRaw();
      auto oldLockedRegistry = lockedRaw.registry;
      /* Lock the manifest, disregarding an existing lockfile. TODO: skip inputs
       * we aren't updating. */
      auto manifestRegistry
        = this->getEnvironment().getManifest().getLockedRegistry();
      /* If no inputs were specified, update everything. */
      if ( ! this->inputNames.has_value() )
        {
          lockedRaw.registry = std::move( manifestRegistry );
        }
      /* If inputs were specified, update each input specified. */
      else
        {
          for ( auto & inputName : *this->inputNames )
            {
              {
                if ( const auto & maybeInput
                     = manifestRegistry.inputs.find( inputName );
                     maybeInput != manifestRegistry.inputs.end() )
                  {
                    lockedRaw.registry.inputs[inputName] = maybeInput->second;
                  }
                else
                  {
                    throw FloxException( "input '" + inputName
                                         + "' does not exist in manifest." );
                  }
              }
            }
          lockedRaw.registry.defaults = manifestRegistry.defaults;
          lockedRaw.registry.priority = manifestRegistry.priority;
        }
      lockfile = lockedRaw;
    }
  /* If the environment doesn't have a lockfile, create one from scratch. Note
   * that even if only some inputs are specified, this will update all inputs.
   */
  else
    {
      // TODO: `RegistryRaw' should drop empty fields.
      lockfile = this->getEnvironment().createLockfile().getLockfileRaw();
    }
  /* Print that bad boii */
  std::cout << lockfile.dump() << '\n';

  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

UpgradeCommand::UpgradeCommand() : parser( "upgrade" )
{
  this->parser.add_description(
    "Upgrade groups or standalone packages in an environment" );
  this->addGlobalManifestFileOption( this->parser );
  this->addLockfileOption( this->parser );
  this->addGARegistryOption( this->parser );

  this->addManifestFileArg( this->parser, false );

  this->parser.add_argument( "groups" )
    .help( "names of groups or standalone packages to upgrade" )
    .metavar( "GROUPS..." )
    .remaining()
    .action(
      [&]( const std::string & groupOrPackageName )
      {
        if ( ! this->groupsOrIIDS.has_value() )
          {
            this->groupsOrIIDS = std::vector<std::string>();
          }
        this->groupsOrIIDS->emplace_back( groupOrPackageName );
      } );
}


/* -------------------------------------------------------------------------- */

// NOLINTBEGIN(readability-function-cognitive-complexity)
int
UpgradeCommand::run()
{
  /* Start by translating groupsOrIIDS to something we can pass to setUpgrades.
   */
  if ( ! this->groupsOrIIDS.has_value() ) { this->setUpgrades( true ); }
  else
    {
      auto manifest           = this->getManifest();
      auto descriptors        = manifest.getDescriptors();
      auto groupedDescriptors = manifest.getGroupedDescriptors();
      std::vector<std::string> groupsToUpgrade;
      for ( const auto & groupOrIID : *this->groupsOrIIDS )
        {
          /* If groupOrIID is a group name, treat it as a group. Note this takes
           * precedence over a package of the same name. */
          if ( groupedDescriptors.find( groupOrIID )
               != groupedDescriptors.end() )
            {
              groupsToUpgrade.emplace_back( groupOrIID );
            }
          /* If groupOrIID is an IID, check if it is the only package in a
           * group. */
          else if ( const auto & maybeDescriptor
                    = descriptors.find( groupOrIID );
                    maybeDescriptor != descriptors.end() )
            {
              const auto & [_, descriptor] = *maybeDescriptor;
              std::string groupName = descriptor.group.value_or( "toplevel" );
              if ( groupedDescriptors.at( groupName ).size() == 1 )
                {
                  groupsToUpgrade.emplace_back( groupName );
                }
              else
                {
                  throw FloxException(
                    nix::fmt( "'%s' is a package in the group '%s' with "
                              "multiple packages.\n"
                              "To upgrade the group, specify the group name:\n"
                              "     $ flox upgrade %s\n"
                              "To upgrade all packages, run:\n"
                              "     $ flox upgrade",
                              groupOrIID,
                              groupName,
                              groupName ) );
                }
            }
          else
            {
              throw FloxException( "'" + groupOrIID
                                   + "' is not a group or key for a package." );
            }
        }
      this->setUpgrades( groupsToUpgrade );
    }

  /* Generate lockfile. */
  Environment environment = this->getEnvironment();
  LockfileRaw newLockfile = environment.createLockfile().getLockfileRaw();

  /* Compare old and new lockfile to generate confirmation message. */
  std::optional<LockfileRaw> oldLockfile;
  if ( auto lockfile = environment.getOldLockfile(); lockfile.has_value() )
    {
      oldLockfile = lockfile->getLockfileRaw();
    }
  std::vector<std::string> upgraded;
  for ( const auto & [system, systemPackages] : newLockfile.packages )
    {
      for ( const auto & [iid, descriptor] : systemPackages )
        {
          if ( oldLockfile.has_value() )
            {
              /* system is in oldLockfile */
              if ( const auto & systemPackagesIterator
                   = oldLockfile->packages.find( system );
                   systemPackagesIterator != oldLockfile->packages.end() )
                {
                  /* iid is in oldLockfile */
                  if ( const auto & maybeOldDescriptor
                       = systemPackagesIterator->second.find( iid );
                       maybeOldDescriptor
                       != systemPackagesIterator->second.end() )
                    {
                      const auto & [_, oldDescriptor] = *maybeOldDescriptor;
                      if ( descriptor != oldDescriptor )
                        {
                          upgraded.emplace_back( iid );
                        }
                      /* else package unchanged */
                    }
                }
            }
        }
    }  // we don't currently print installs or uninstalls

  /* Print that bad boii */
  nlohmann::json result
    = { { "lockfile", newLockfile }, { "result", upgraded } };
  std::cout << result.dump() << '\n';

  return EXIT_SUCCESS;
}
// NOLINTEND(readability-function-cognitive-complexity)

/* -------------------------------------------------------------------------- */

RegistryCommand::RegistryCommand() : parser( "registry" )
{
  this->parser.add_description( "Show environment registry information" );
  this->addGlobalManifestFileOption( this->parser );
  this->addLockfileOption( this->parser );
  this->addGARegistryOption( this->parser );
  /* TODO: make manifest file optional and support showing global manifest
   * registry. */
  this->addManifestFileArg( this->parser, true );
}


/* -------------------------------------------------------------------------- */

int
RegistryCommand::run()
{
  nlohmann::json registries = {
    { "manifest", this->getEnvironment().getManifest().getRegistryRaw() },
    { "manifest-locked",
      this->getEnvironment().getManifest().getLockedRegistry() },
    { "combined", this->getEnvironment().getCombinedRegistryRaw() },
  };

  if ( auto maybeGlobal = this->getEnvironment().getGlobalManifest();
       maybeGlobal.has_value() )
    {
      registries["global"]        = maybeGlobal->getRegistryRaw();
      registries["global-locked"] = maybeGlobal->getLockedRegistry();
    }
  else
    {
      registries["global"]        = nullptr;
      registries["global-locked"] = nullptr;
    }

  if ( auto maybeLock = this->getEnvironment().getOldLockfile();
       maybeLock.has_value() )
    {
      registries["lockfile"]          = maybeLock->getRegistryRaw();
      registries["lockfile-packages"] = maybeLock->getPackagesRegistryRaw();
    }
  else
    {
      registries["lockfile"]          = nullptr;
      registries["lockfile-packages"] = nullptr;
    }

  std::cout << registries.dump() << '\n';
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

CheckCommand::CheckCommand() : parser( "check" )
{
  this->parser.add_description(
    "Lint a manifest and return warnings as newline delimited json objects" );
  this->parser.add_argument( "--lockfile" )
    .help( "the path to the project's 'manifest.lock'" )
    .metavar( "PATH" )
    .nargs( 1 )
    .action( [&]( const std::string & strPath )
             { this->setLockfileRaw( nix::absPath( strPath ) ); } );

  this->parser.add_argument( "--system" )
    .help( "The system to check packages for" )
    .metavar( "SYSTEM" )
    .nargs( 1 )
    .action( [&]( const std::string & system ) { this->system = system; } );
}

void
CheckCommand::setLockfileRaw( const std::filesystem::path & path )
{
  if ( this->lockfile.has_value() )
    {
      throw EnvironmentMixinException(
        "lockfile already initialized, cannot change." );
    }

  if ( ! std::filesystem::exists( path ) )
    {
      throw InvalidLockfileException( "lockfile '" + path.string()
                                      + "' does not exist." );
    }

  this->lockfileRaw = readAndCoerceJSON( path );
}

Lockfile
CheckCommand::getLockfile()
{
  if ( this->lockfile.has_value() ) { return *this->lockfile; }

  if ( ! this->lockfileRaw.has_value() )
    {
      throw EnvironmentMixinException(
        "lockfile not initialized, cannot get." );
    }

  this->lockfile = Lockfile( *this->lockfileRaw );
  return *this->lockfile;
}

int
CheckCommand::run()
{
  auto warnings = this->getLockfile().checkPackages(
    this->system.value_or( nix::nativeSystem ) );

  std::cout << nlohmann::json( warnings ).dump() << std::endl;

  return EXIT_SUCCESS;
}

/* -------------------------------------------------------------------------- */

ManifestCommand::ManifestCommand() : parser( "manifest" )
{
  this->parser.add_description( "Manifest subcommands" );
  this->parser.add_subparser( this->cmdLock.getParser() );
  this->parser.add_subparser( this->cmdDiff.getParser() );
  this->parser.add_subparser( this->cmdRegistry.getParser() );
  this->parser.add_subparser( this->cmdUpdate.getParser() );
  this->parser.add_subparser( this->cmdUpgrade.getParser() );
  this->parser.add_subparser( this->cmdCheck.getParser() );
}


/* -------------------------------------------------------------------------- */

int
ManifestCommand::run()
{
  if ( this->parser.is_subcommand_used( "lock" ) )
    {
      return this->cmdLock.run();
    }
  if ( this->parser.is_subcommand_used( "diff" ) )
    {
      return this->cmdDiff.run();
    }
  if ( this->parser.is_subcommand_used( "update" ) )
    {
      return this->cmdUpdate.run();
    }
  if ( this->parser.is_subcommand_used( "upgrade" ) )
    {
      return this->cmdUpgrade.run();
    }
  if ( this->parser.is_subcommand_used( "registry" ) )
    {
      return this->cmdRegistry.run();
    }
  if ( this->parser.is_subcommand_used( "check" ) )
    {
      return this->cmdCheck.run();
    }
  std::cerr << this->parser << '\n';
  throw flox::FloxException( "You must provide a valid 'manifest' subcommand" );
  return EXIT_FAILURE;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
