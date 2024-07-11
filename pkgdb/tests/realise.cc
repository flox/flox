/* ========================================================================== *
 *
 *  @file realise.cc
 *
 *  @brief Tests for `buildenv::realise` functionality.
 *
 * -------------------------------------------------------------------------- */

#include "flox/buildenv/realise.hh"
#include "flox/buildenv/buildenv-lockfile.hh"
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

/* Create a BuildenvLockfile with profile and hook sections in the manifest */
flox::buildenv::BuildenvLockfile
testLockfile()
{
  std::string                 json         = R"({
    "profile": {
      "common": "echo hello",
      "bash": "echo hello",
      "fish": "echo hello",
      "tcsh": "echo hello",
      "zsh": "echo hello"
    },
    "hook": {
      "on-activate": "echo hello"
    }
  })";
  nlohmann::json              manifestJson = nlohmann::json::parse( json );
  flox::resolver::ManifestRaw manifestRaw;
  from_json( manifestJson, manifestRaw );
  flox::buildenv::BuildenvLockfile lockfile = flox::buildenv::BuildenvLockfile {
    .manifest = manifestRaw,
    .packages = {},
  };
  return lockfile;
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
test_scriptsAreAddedToScriptsDir( nix::ref<nix::EvalState> &         state,
                                  flox::buildenv::BuildenvLockfile & lockfile )
{
  auto output     = flox::buildenv::makeActivationScripts( *state, lockfile );
  auto scriptsDir = std::filesystem::path( output.first.path )
                    / flox::buildenv::ACTIVATION_SUBDIR_NAME;
  std::vector<std::string> scripts
    = { "profile-common", "profile-bash", "profile-zsh",
        "profile-fish",   "profile-tcsh", "hook-on-activate" };
  for ( const auto & script : scripts )
    {
      auto path = scriptsDir / script;
      EXPECT( std::filesystem::exists( path ) );
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

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
