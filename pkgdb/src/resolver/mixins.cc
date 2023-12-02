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


/* -------------------------- non virtual set/get --------------------------- */

void
EnvironmentMixin::setGlobalManifestRaw(
  std::optional<std::filesystem::path> maybePath )
{

  if ( ! maybePath.has_value() )
    {
      this->globalManifestRaw = std::nullopt;
      return;
    }

  if ( this->globalManifest.has_value() )
    {
      throw EnvironmentMixinException(
        "global manifest path cannot be set after global manifest was "
        "initialized" );
    }

  this->globalManifestRaw
    = readManifestFromPath<GlobalManifestRaw>( *maybePath );
}

void
EnvironmentMixin::setGlobalManifestRaw(
  std::optional<GlobalManifestRaw> maybeRaw )
{

  if ( ! maybeRaw.has_value() )
    {
      this->globalManifestRaw = std::nullopt;
      return;
    }

  if ( this->globalManifest.has_value() )
    {
      throw EnvironmentMixinException(
        "global manifest path cannot be set after global manifest was "
        "initialized" );
    }

  this->globalManifestRaw = std::move( maybeRaw );
}


const std::optional<GlobalManifest>
EnvironmentMixin::getGlobalManifest()
{
  if ( ! this->globalManifest.has_value() )
    {
      auto manifestRaw = this->getGlobalManifestRaw();
      if ( ! manifestRaw.has_value() ) { return std::nullopt; }
      this->globalManifest = this->initGlobalManifest( *manifestRaw );
    }

  return this->globalManifest;
}


/* -------------------------------------------------------------------------- */

void
EnvironmentMixin::setManifestRaw(
  std::optional<std::filesystem::path> maybePath )
{

  if ( ! maybePath.has_value() )
    {
      this->manifestRaw = std::nullopt;
      return;
    }

  if ( this->manifest.has_value() )
    {
      throw EnvironmentMixinException(
        " manifest path cannot be set after  manifest was "
        "initialized" );
    }

  this->manifestRaw = readManifestFromPath<ManifestRaw>( *maybePath );
}

void
EnvironmentMixin::setManifestRaw( std::optional<ManifestRaw> maybeRaw )
{

  if ( ! maybeRaw.has_value() )
    {
      this->manifestRaw = std::nullopt;
      return;
    }

  if ( this->manifest.has_value() )
    {
      throw EnvironmentMixinException(
        " manifest path cannot be set after  manifest was "
        "initialized" );
    }

  this->manifestRaw = std::move( maybeRaw );
}

const EnvironmentManifest &
EnvironmentMixin::getManifest()
{
  if ( ! this->manifest.has_value() )
    {
      if ( auto manifestRaw = this->getManifestRaw(); manifestRaw.has_value() )
        {
          this->manifest = this->initManifest( *manifestRaw );
        }
      else
        {
          throw EnvironmentMixinException(
            "raw manifest or manifest path must be set before manifest can be "
            "initialized" );
        }
    }

  return *this->manifest;
}


/* -------------------------------------------------------------------------- */

void
EnvironmentMixin::setLockfileRaw( std::filesystem::path path )
{
  if ( ! std::filesystem::exists( path ) )
    {
      throw InvalidLockfileException( "no such path: " + path.string() );
    }

  LockfileRaw lockfileRaw = readAndCoerceJSON( path );
  this->setLockfileRaw( lockfileRaw );
}

void
EnvironmentMixin::setLockfileRaw( LockfileRaw lockfileRaw )
{
  if ( this->lockfile.has_value() )
    {
      throw EnvironmentMixinException(
        "lockfile path cannot be set after lockfile was "
        "initialized" );
    }
  this->lockfileRaw = std::move( lockfileRaw );
}

const std::optional<Lockfile> &
EnvironmentMixin::getLockfile()
{
  if ( ! this->lockfile.has_value() )
    {
      if ( auto lockfileRaw = this->getLockfileRaw(); lockfileRaw.has_value() )
        {
          this->lockfile = this->initLockfile( *lockfileRaw );
        }
    }

  return this->lockfile;
}

/* -------------------------------------------------------------------------- */

Lockfile
EnvironmentMixin::initLockfile( LockfileRaw lockfileRaw )
{
  return Lockfile( std::move( lockfileRaw ) );
}


/* ------------------------ virtual specific impls -------------------------- */

GlobalManifest
EnvironmentMixin::initGlobalManifest( GlobalManifestRaw manifestRaw )
{
  return GlobalManifest( std::move( manifestRaw ) );
}


/* -------------------------------------------------------------------------- */

EnvironmentManifest
EnvironmentMixin::initManifest( ManifestRaw manifestRaw )
{
  return EnvironmentManifest( std::move( manifestRaw ) );
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
             { this->setGlobalManifestRaw( nix::absPath( strPath ) ); } );
}


/* -------------------------------------------------------------------------- */

argparse::Argument &
EnvironmentMixin::addManifestFileOption( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "--manifest" )
    .help( "the path to the `manifest.{toml,yaml,json}' file." )
    .metavar( "PATH" )
    .action( [&]( const std::string & strPath )
             { this->setManifestRaw( nix::absPath( strPath ) ); } );
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
                 { this->setManifestRaw( nix::absPath( strPath ) ); } );
  return required ? arg.required() : arg;
}


/* ---------------------- EnvironmentMixin arguments ------------------------ */

argparse::Argument &
EnvironmentMixin::addLockfileOption( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "--lockfile" )
    .help( "the path to the projects existing `manifest.lock' file." )
    .metavar( "PATH" )
    .action( [&]( const std::string & strPath )
             { this->setLockfileRaw( nix::absPath( strPath ) ); } );
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
        if ( std::filesystem::exists( path ) ) { this->setLockfileRaw( path ); }

        /* Locate manifest. */
        // NOLINTBEGIN(bugprone-branch-clone)
        if ( path = dir / "manifest.json"; std::filesystem::exists( path ) )
          {
            this->setManifestRaw( path );
          }
        else if ( path = dir / "manifest.toml";
                  std::filesystem::exists( path ) )
          {
            this->setManifestRaw( path );
          }
        else if ( path = dir / "manifest.yaml";
                  std::filesystem::exists( path ) )
          {
            this->setManifestRaw( path );
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

/* ---------------- EnvironmentMixin init overrides ------------------------- */

GlobalManifest
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
  return this->EnvironmentMixin::initGlobalManifest( manifestRaw );
}

/* -------------------------------------------------------------------------- */

EnvironmentManifest
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
  return this->EnvironmentMixin::initManifest( manifestRaw );
}


/* --------------------- GAEnvironmentMixin arguments ----------------------- */

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
