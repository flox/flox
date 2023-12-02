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
  this->addManifestFileArg( this->parser );
}


/* -------------------------------------------------------------------------- */

int
LockCommand::run()
{
  // TODO: `RegistryRaw' should drop empty fields.
  nlohmann::json lockfile
    = this->getEnvironment().createLockfile().getLockfileRaw();
  /* Print that bad boii */
  std::cout << lockfile.dump() << std::endl;
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
          throw InvalidManifestFileException( "manifest file `"
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
          throw InvalidManifestFileException( "old manifest file `"
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
  std::cout << diff.dump() << std::endl;
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

UpdateCommand::UpdateCommand() : parser( "update" )
{
  this->parser.add_description( "Update environment inputs" );
  this->addGlobalManifestFileOption( this->parser );
  this->addLockfileOption( this->parser );
  this->addGARegistryOption( this->parser );

  this->parser.add_argument( "-i", "--input" )
    .help( "name of input to update" )
    .nargs( 1 )
    .metavar( "NAME" )
    .action( [&]( const std::string & inputName )
             { this->inputName = inputName; } );

  this->addManifestFileArg( this->parser );
}


/* -------------------------------------------------------------------------- */

int
UpdateCommand::run()
{
  if ( auto maybeLockfile = this->getLockfile(); maybeLockfile.has_value() )
    {
      auto lockedRaw = maybeLockfile->getLockfileRaw();
      auto manifestRegistry
        = this->getEnvironment().getManifest().getLockedRegistry();
      if ( this->inputName.has_value() )
        {
          if ( const auto & maybeInput
               = manifestRegistry.inputs.find( *this->inputName );
               maybeInput != manifestRegistry.inputs.end() )
            {
              lockedRaw.registry.inputs[*this->inputName] = maybeInput->second;
              lockedRaw.registry.defaults = manifestRegistry.defaults;
              lockedRaw.registry.priority = manifestRegistry.priority;
            }
          else
            {
              throw FloxException( "input `" + *this->inputName
                                   + "' does not exist in manifest." );
            }
        }
      else { lockedRaw.registry = std::move( manifestRegistry ); }
      std::cout << nlohmann::json( lockedRaw ).dump() << std::endl;
    }
  else
    {
      // TODO: `RegistryRaw' should drop empty fields.
      nlohmann::json lockfile
        = this->getEnvironment().createLockfile().getLockfileRaw();
      /* Print that bad boii */
      std::cout << lockfile.dump() << std::endl;
    }
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

RegistryCommand::RegistryCommand() : parser( "registry" )
{
  this->parser.add_description( "Show environment registry information" );
  this->addGlobalManifestFileOption( this->parser );
  this->addLockfileOption( this->parser );
  this->addGARegistryOption( this->parser );
  this->addManifestFileArg( this->parser );
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

  std::cout << registries.dump() << std::endl;
  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- */

ManifestCommand::ManifestCommand() : parser( "manifest" ), cmdLock(), cmdDiff()
{
  this->parser.add_description( "Manifest subcommands" );
  this->parser.add_subparser( this->cmdLock.getParser() );
  this->parser.add_subparser( this->cmdDiff.getParser() );
  this->parser.add_subparser( this->cmdRegistry.getParser() );
  this->parser.add_subparser( this->cmdUpdate.getParser() );
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
  if ( this->parser.is_subcommand_used( "registry" ) )
    {
      return this->cmdRegistry.run();
    }
  std::cerr << this->parser << std::endl;
  throw flox::FloxException( "You must provide a valid `manifest' subcommand" );
  return EXIT_FAILURE;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
