/* ========================================================================== *
 *
 * @file resolver/mixins.cc
 *
 * @brief State blobs for flox commands.
 *
 *
 * -------------------------------------------------------------------------- */

#include <filesystem>
#include <optional>
#include <string>
#include <string_view>
#include <utility>
#include <variant>

#include <argparse/argparse.hpp>
#include <nix/util.hh>

#include "flox/resolver/environment.hh"
#include "flox/resolver/lockfile.hh"
#include "flox/resolver/manifest-raw.hh"
#include "flox/resolver/manifest.hh"
#include "flox/resolver/mixins.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

/**
 * @brief Generate exception handling boilerplate for
 *        `EnvironmentMixin::init<MEMBER>' functions.
 */
#define ENV_MIXIN_THROW_IF_SET( member )                              \
  if ( this->member.has_value() )                                     \
    {                                                                 \
      throw EnvironmentMixinException( "`" #member                    \
                                       "' was already initialized" ); \
    }                                                                 \
  if ( this->environment.has_value() )                                \
    {                                                                 \
      throw EnvironmentMixinException(                                \
        "`" #member "' cannot be initialized after `environment'" );  \
    }


/* -------------------------------------------------------------------------- */

void
EnvironmentMixin::initGlobalManifest( GlobalManifestRaw manifestRaw )
{
  ENV_MIXIN_THROW_IF_SET( globalManifest )
  this->globalManifest = GlobalManifest( std::move( manifestRaw ) );
}

void
EnvironmentMixin::initGlobalManifestPath( std::filesystem::path path )
{
  ENV_MIXIN_THROW_IF_SET( globalManifestPath )
  this->globalManifestPath = std::move( path );
  this->initGlobalManifest(
    readManifestFromPath<GlobalManifestRaw>( *this->globalManifestPath ) );
}


/* -------------------------------------------------------------------------- */

void
EnvironmentMixin::initManifest( ManifestRaw manifestRaw )
{
  ENV_MIXIN_THROW_IF_SET( manifest )
  this->manifest = EnvironmentManifest( std::move( manifestRaw ) );
}

void
EnvironmentMixin::initManifestPath( std::filesystem::path path )
{
  ENV_MIXIN_THROW_IF_SET( manifestPath )
  this->manifestPath = std::move( path );
  this->initManifest(
    readManifestFromPath<ManifestRaw>( *this->manifestPath ) );
}


/* -------------------------------------------------------------------------- */

void
EnvironmentMixin::initLockfile( LockfileRaw lockfileRaw )
{
  ENV_MIXIN_THROW_IF_SET( lockfile );
  this->lockfile = Lockfile( std::move( lockfileRaw ) );
}

void
EnvironmentMixin::initLockfilePath( std::filesystem::path path )
{
  if ( ! std::filesystem::exists( path ) )
    {
      throw InvalidLockfileException( "no such path: " + path.string() );
    }
  ENV_MIXIN_THROW_IF_SET( lockfilePath )
  this->lockfilePath = std::move( path );
  this->initLockfile( readAndCoerceJSON( *this->lockfilePath ) );
}


/* -------------------------------------------------------------------------- */

const EnvironmentManifest &
EnvironmentMixin::getManifest()
{
  if ( ! this->manifest.has_value() )
    {
      throw EnvironmentMixinException(
        "you must provide an inline manifest or the path to a "
        "manifest file" );
    }
  return *this->manifest;
}


/* -------------------------------------------------------------------------- */

Environment &
EnvironmentMixin::getEnvironment()
{
  if ( ! this->environment.has_value() )
    {
      this->environment
        = std::make_optional<Environment>( this->getGlobalManifest(),
                                           this->getManifest(),
                                           this->getLockfile() );
    }
  return *this->environment;
}


/* -------------------------------------------------------------------------- */

argparse::Argument &
EnvironmentMixin::addGlobalManifestFileOption(
  argparse::ArgumentParser & parser )
{
  return parser.add_argument( "--global-manifest" )
    .help( "the path to the user's global `manifest.{toml,yaml,json}' file." )
    .metavar( "PATH" )
    .action( [&]( const std::string & strPath )
             { this->initGlobalManifestPath( nix::absPath( strPath ) ); } );
}


/* -------------------------------------------------------------------------- */

argparse::Argument &
EnvironmentMixin::addManifestFileOption( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "--manifest" )
    .help( "the path to the `manifest.{toml,yaml,json}' file." )
    .metavar( "PATH" )
    .action( [&]( const std::string & strPath )
             { this->initManifestPath( nix::absPath( strPath ) ); } );
}


/* -------------------------------------------------------------------------- */

argparse::Argument &
EnvironmentMixin::addManifestFileArg( argparse::ArgumentParser & parser,
                                      bool                       required )
{
  argparse::Argument & arg
    = parser.add_argument( "manifest" )
        .help( "the path to the project's `manifest.{toml,yaml,json}' file." )
        .metavar( "MANIFEST-PATH" )
        .action( [&]( const std::string & strPath )
                 { this->initManifestPath( nix::absPath( strPath ) ); } );
  return required ? arg.required() : arg;
}


/* -------------------------------------------------------------------------- */

argparse::Argument &
EnvironmentMixin::addLockfileOption( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "--lockfile" )
    .help( "the path to the projects existing `manifest.lock' file." )
    .metavar( "PATH" )
    .action( [&]( const std::string & strPath )
             { this->initLockfilePath( nix::absPath( strPath ) ); } );
}


/* -------------------------------------------------------------------------- */

argparse::Argument &
EnvironmentMixin::addFloxDirectoryOption( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "--dir", "-d" )
    .help( "the directory to search for `manifest.{json,yaml,toml}' and "
           "`manifest.lock`." )
    .metavar( "PATH" )
    .nargs( 1 )
    .action(
      [&]( const std::string & strPath )
      {
        std::filesystem::path dir( nix::absPath( strPath ) );
        /* Try to locate lockfile. */
        auto path = dir / "manifest.lock";
        if ( std::filesystem::exists( path ) )
          {
            this->initLockfilePath( path );
          }

        /* Locate manifest. */
        // NOLINTBEGIN(bugprone-branch-clone)
        if ( path = dir / "manifest.json"; std::filesystem::exists( path ) )
          {
            this->initManifestPath( path );
          }
        else if ( path = dir / "manifest.toml";
                  std::filesystem::exists( path ) )
          {
            this->initManifestPath( path );
          }
        else if ( path = dir / "manifest.yaml";
                  std::filesystem::exists( path ) )
          {
            this->initManifestPath( path );
          }
        else
          {
            throw EnvironmentMixinException(
              "unable to locate a `manifest.{json,yaml,toml}' file "
              "in directory: "
              + strPath );
          }
        // NOLINTEND(bugprone-branch-clone)
      } );
}


/* -------------------------------------------------------------------------- */

void
GAEnvironmentMixin::initGlobalManifest( GlobalManifestRaw manifestRaw )
{
  if ( this->gaRegistry )
    {
      (void) static_cast<GlobalManifestRawGA>( manifestRaw );
      if ( ! manifestRaw.registry.has_value() )
        {
          manifestRaw.registry = getGARegistry();
        }
    }
  this->EnvironmentMixin::initGlobalManifest( std::move( manifestRaw ) );
}


/* -------------------------------------------------------------------------- */

void
GAEnvironmentMixin::initGlobalManifestPath( std::filesystem::path path )
{
  if ( this->gaRegistry )
    {
      auto manifestRawGA = readManifestFromPath<GlobalManifestRawGA>( path );
      auto manifestRaw   = static_cast<GlobalManifestRaw>( manifestRawGA );
      this->setGlobalManifestPath( std::move( path ) );
      this->EnvironmentMixin::initGlobalManifest( std::move( manifestRaw ) );
    }
  else { this->EnvironmentMixin::initGlobalManifestPath( std::move( path ) ); }
}


/* -------------------------------------------------------------------------- */

void
GAEnvironmentMixin::initManifest( ManifestRaw manifestRaw )
{
  if ( this->gaRegistry )
    {
      (void) static_cast<ManifestRawGA>( manifestRaw );
      if ( ! manifestRaw.registry.has_value() )
        {
          manifestRaw.registry = getGARegistry();
        }
    }
  this->EnvironmentMixin::initManifest( std::move( manifestRaw ) );
}


/* -------------------------------------------------------------------------- */

void
GAEnvironmentMixin::initManifestPath( std::filesystem::path path )
{
  if ( this->gaRegistry )
    {
      auto manifestRawGA = readManifestFromPath<ManifestRawGA>( path );
      auto manifestRaw   = static_cast<ManifestRaw>( manifestRawGA );
      this->setManifestPath( std::move( path ) );
      this->EnvironmentMixin::initManifest( std::move( manifestRaw ) );
    }
  else { this->EnvironmentMixin::initManifestPath( std::move( path ) ); }
}


/* -------------------------------------------------------------------------- */

const std::optional<GlobalManifest> &
GAEnvironmentMixin::getGlobalManifest()
{
  if ( ( ! this->EnvironmentMixin::getGlobalManifest().has_value() )
       && this->gaRegistry )
    {
      this->initGlobalManifest( GlobalManifestRaw() );
    }
  return this->EnvironmentMixin::getGlobalManifest();
}


/* -------------------------------------------------------------------------- */

argparse::Argument &
GAEnvironmentMixin::addGARegistryOption( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "--ga-registry" )
    .help( "use a hard coded manifest ( for `flox' GA )." )
    .nargs( 0 )
    .action( [&]( const auto & ) { this->gaRegistry = true; } );
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
