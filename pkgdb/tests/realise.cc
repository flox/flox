/* ========================================================================== *
 *
 *  @file realise.cc
 *
 *  @brief Tests for `buildenv::realise` functionality.
 *
 * -------------------------------------------------------------------------- */

#include "flox/buildenv/realise.hh"
#include "flox/core/util.hh"
#include "flox/resolver/environment.hh"
#include "flox/resolver/manifest.hh"
#include "test.hh"
#include <fstream>
#include <nix/flake/flake.hh>

/* -------------------------------------------------------------------------- */


nix::ref<nix::eval_cache::AttrCursor>
cursorForPackageName( nix::ref<nix::EvalState> & state,
                      const std::string &        system,
                      const std::string &        name )
{
  auto flakeRef = nix::parseFlakeRef( nixpkgsRef );
  auto lockedRef
    = nix::flake::lockFlake( *state, flakeRef, nix::flake::LockFlags {} );
  std::vector<std::string> attrPath = { "legacyPackages", system, name };
  auto cursor = flox::buildenv::getPackageCursor( state, lockedRef, attrPath );
  return cursor;
}


/* -------------------------------------------------------------------------- */

std::string
unsupportedPackage( const std::string & system )
{
  if ( system == "aarch64-darwin" ) { return "glibc"; }
  else if ( system == "x86_64-darwin" ) { return "glibc"; }
  else if ( system == "aarch64-linux" ) { return "spacebar"; }
  else if ( system == "x86_64-linux" ) { return "spacebar"; }
  else
    {
      // Should be unreachable
      return "wat?";
    }
}


/* -------------------------------------------------------------------------- */

/* Create lockfile from a manifest with profile and hook sections */
flox::resolver::Lockfile
testLockfile()
{
  std::string                 json         = R"({
    "profile": {
      "common": "echo hello",
      "bash": "echo hello",
      "zsh": "echo hello"
    },
    "hook": {
      "on-activate": "echo hello"
    }
  })";
  nlohmann::json              manifestJson = nlohmann::json::parse( json );
  flox::resolver::ManifestRaw manifestRaw;
  from_json( manifestJson, manifestRaw );
  flox::resolver::EnvironmentManifest manifest( manifestRaw );
  flox::resolver::Environment         env( manifest );
  return env.createLockfile();
}

/* -------------------------------------------------------------------------- */

bool
test_tryEvaluatePackageOutPathReturnsValidOutpath(
  nix::ref<nix::EvalState> & state,
  const std::string &        system )
{
  auto pkg    = "ripgrep";
  auto cursor = cursorForPackageName( state, system, pkg );
  auto path
    = flox::buildenv::tryEvaluatePackageOutPath( state, pkg, system, cursor

    );
  auto storePath = state->store->maybeParseStorePath( path );

  return storePath.has_value();
}


/* -------------------------------------------------------------------------- */

bool
test_evalFailureForInsecurePackage( nix::ref<nix::EvalState> & state,
                                    const std::string &        system )
{
  auto pkg    = "python2";
  auto cursor = cursorForPackageName( state, system, pkg );
  try
    {
      auto path = flox::buildenv::tryEvaluatePackageOutPath( state,
                                                             pkg,
                                                             system,
                                                             cursor );
      return false;
    }
  catch ( const flox::buildenv::PackageEvalFailure & )
    {
      return true;
    }
  catch ( const std::exception & )
    {
      return false;
    }
}


/* -------------------------------------------------------------------------- */

bool
test_unsupportedSystemExceptionForUnsupportedPackage(
  nix::ref<nix::EvalState> & state,
  const std::string &        system )
{
  auto pkg    = unsupportedPackage( system );
  auto cursor = cursorForPackageName( state, system, pkg );
  try
    {
      auto path = flox::buildenv::tryEvaluatePackageOutPath( state,
                                                             pkg,
                                                             system,
                                                             cursor );
      return false;
    }
  catch ( const flox::buildenv::PackageUnsupportedSystem & )
    {
      return true;
    }
  catch ( const std::exception & )
    {
      return false;
    }
}


/* -------------------------------------------------------------------------- */

bool
test_scriptsAreAddedToScriptsDir( nix::ref<nix::EvalState> & state,
                                  flox::resolver::Lockfile & lockfile )
{
  auto output     = flox::buildenv::makeActivationScripts( *state, lockfile );
  auto scriptsDir = std::filesystem::path( output.first.path )
                    / flox::buildenv::ACTIVATION_SUBDIR_NAME;
  std::vector<std::string> scripts
    = { "profile-common",   "profile-bash", "profile-zsh",
        "hook-on-activate", "bash",         "zsh" };
  for ( const auto & script : scripts )
    {
      auto path = scriptsDir / script;
      EXPECT( std::filesystem::exists( path ) );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_scriptsAreSourcedOrCalled( nix::ref<nix::EvalState> & state,
                                flox::resolver::Lockfile & lockfile )
{
  auto output     = flox::buildenv::makeActivationScripts( *state, lockfile );
  auto scriptsDir = std::filesystem::path( output.first.path )
                    / flox::buildenv::ACTIVATION_SUBDIR_NAME;
  std::vector<std::string> shells         = { "bash", "zsh" };
  std::vector<std::string> profileScripts = { "common" };
  profileScripts.insert( profileScripts.begin(), shells.begin(), shells.end() );
  for ( const auto & shell : shells )
    {
      auto              scriptPath = scriptsDir / shell;
      std::ifstream     file( scriptPath );
      std::stringstream contents;
      contents << file.rdbuf();
      file.close();

      /* Look for 'profile-common'*/
      auto commonPattern = nix::fmt( "\"$FLOX_ENV/%s/profile-%s\"",
                                     flox::buildenv::ACTIVATION_SUBDIR_NAME,
                                     "common" );
      auto commonPos     = contents.str().find( commonPattern );
      EXPECT( commonPos != std::string::npos );

      /* Look for 'profile-<shell>'*/
      auto shellPattern = nix::fmt( "\"$FLOX_ENV/%s/profile-%s\"",
                                    flox::buildenv::ACTIVATION_SUBDIR_NAME,
                                    shell );
      auto shellPos     = contents.str().find( shellPattern );
      EXPECT( shellPos != std::string::npos );

      /* Look for 'hook-on-activate'*/
      auto hookPattern = nix::fmt( "bash \"$FLOX_ENV/%s/hook-on-activate\"",
                                   flox::buildenv::ACTIVATION_SUBDIR_NAME );
      auto hookPos     = contents.str().find( hookPattern );
      EXPECT( hookPos != std::string::npos );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

int
main( int argc, char * argv[] )
{
  int exitCode = EXIT_SUCCESS;
#define RUN_TEST( ... ) _RUN_TEST( exitCode, __VA_ARGS__ )

  nix::verbosity = nix::lvlWarn;
  if ( ( 1 < argc ) && ( std::string_view( argv[1] ) == "-v" ) )  // NOLINT
    {
      nix::verbosity = nix::lvlDebug;
    }

  /* Initialize `nix' */
  flox::NixState nstate;
  auto           state = nstate.getState();

  std::string system = nix::nativeSystem;

  RUN_TEST( tryEvaluatePackageOutPathReturnsValidOutpath, state, system );
  RUN_TEST( evalFailureForInsecurePackage, state, system );
  RUN_TEST( unsupportedSystemExceptionForUnsupportedPackage, state, system );

  auto lockfile = testLockfile();

  RUN_TEST( scriptsAreAddedToScriptsDir, state, lockfile );
  RUN_TEST( scriptsAreSourcedOrCalled, state, lockfile );

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
