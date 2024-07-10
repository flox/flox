/* ========================================================================== *
 *
 *  @file lock-flake-installable.cc
 *
 *  @brief Tests for `lock-flake-installable` functionality.
 *
 * -------------------------------------------------------------------------- */

#include "flox/lock-flake-installable.hh"
#include "flox/core/util.hh"
#include "test.hh"
#include <fstream>
#include <nix/flake/flake.hh>

/* -------------------------------------------------------------------------- */


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

/**
 * paths are relative to the test runner which in this case is the makefile in
 * the pkgdb root
 */
const std::string localTestFlake
  = std::filesystem::absolute( "./tests/data/lock-flake-installable" ).string();


/* -------------------------------------------------------------------------- */

bool
test_attrpathUsesDefaults( const nix::ref<nix::EvalState> & state,
                           const std::string &              system )
{
  auto lockedExplicit = flox::lockFlakeInstallable(
    state,
    system,
    localTestFlake + "#packages." + system + ".hello" );

  auto lockedImplicit
    = flox::lockFlakeInstallable( state, system, localTestFlake + "#hello" );

  EXPECT_EQ( nlohmann::json( lockedExplicit ),
             nlohmann::json( lockedImplicit ) );

  EXPECT_EQ( lockedImplicit.lockedFlakeAttrPath,
             "packages." + system + ".hello" );

  return true;
}

/**
 * @brief Test that the flake origin is correctly parsed from the flake
 */
bool
test_flakerefOrigins( const nix::ref<nix::EvalState> & state,
                      const std::string &              system )
{
  auto githubScheme = flox::lockFlakeInstallable(
    state,
    system,
    "github:nixos/nixpkgs/6861ef7707a56725769594aaa725518cbe65f628#hello" );
  auto gitHttpsScheme
    = flox::lockFlakeInstallable( state,
                                  system,
                                  "git+https://github.com/flox/flox" );

  auto gitFileScheme
    = flox::lockFlakeInstallable( state,
                                  system,
                                  "path:" + localTestFlake + "#hello" );

  auto gitFileSchemeImplied
    = flox::lockFlakeInstallable( state, system, localTestFlake + "#hello" );


  return true;
}

/**
 * @brief Test that the flake origin is correctly parsed from the flake
 */
bool
test_locksUrl( const nix::ref<nix::EvalState> & state,
               const std::string &              system )
{
  auto unlockedUrl = localTestFlake + "#hello";
  auto lockedInstallable
    = flox::lockFlakeInstallable( state, system, unlockedUrl );

  EXPECT( nix::parseFlakeRef( lockedInstallable.lockedUrl ).input.isLocked() );

  return true;
}

bool
test_explicitOutputs( const nix::ref<nix::EvalState> & state,
                      const std::string &              system )
{

  auto defaultOutputs
    = flox::lockFlakeInstallable( state,
                                  system,
                                  localTestFlake + "#multipleOutputs" );

  // Default outputs of openssl are `bin` and `man`
  EXPECT_EQ( nlohmann::json( defaultOutputs.outputsToInstall ),
             nlohmann::json( nix::StringSet( { "out", "man" } ) ) );

  EXPECT( ! defaultOutputs.requestedOutputsToInstall.has_value() )

  auto explicitOutputs
    = flox::lockFlakeInstallable( state,
                                  system,
                                  localTestFlake + "#multipleOutputs^man,dev" );

  EXPECT_EQ( nlohmann::json( explicitOutputs.requestedOutputsToInstall ),
             nlohmann::json( nix::StringSet( { "man", "dev" } ) ) );

  auto allOutputs
    = flox::lockFlakeInstallable( state,
                                  system,
                                  localTestFlake + "#multipleOutputs^*" );


  EXPECT_EQ( nlohmann::json( allOutputs.requestedOutputsToInstall ),
             nlohmann::json( nix::StringSet( { "out", "man", "dev" } ) ) );

  return true;
}

/**
 * @brief Test that the default package is resolved correctly if no attrpath is
 * provided
 */
bool
test_resolvesToDefaultPackage( const nix::ref<nix::EvalState> & state,
                               const std::string &              system )
{
  auto defaultPackage
    = flox::lockFlakeInstallable( state, system, localTestFlake );

  auto explicitPackage = flox::lockFlakeInstallable(
    state,
    system,
    localTestFlake + "#packages." + system + ".default" );

  EXPECT_EQ( nlohmann::json( defaultPackage ),
             nlohmann::json( explicitPackage ) );

  return true;
}

/**
 * @brief Test that the default package is resolved correctly if no attrpath is
 * provided
 */
bool
test_systemAttributes( const nix::ref<nix::EvalState> & state )
{

  // Test that the system is correctly determined from the attrpath,
  // and locking system is also present in lock
  auto systemSpecifiedInAttrpath = flox::lockFlakeInstallable(
    state,
    "aarch64-linux",
    localTestFlake + "#packages.aarch64-darwin.hello" );

  EXPECT_EQ( systemSpecifiedInAttrpath.packageSystem, "aarch64-darwin" );
  EXPECT_EQ( systemSpecifiedInAttrpath.lockedSystem, "aarch64-linux" );

  return true;
}


bool
test_licenseString( const nix::ref<nix::EvalState> & state,
                    const std::string &              system )
{
  auto licenseString
    = flox::lockFlakeInstallable( state,
                                  system,
                                  localTestFlake + "#licenseString" );

  EXPECT( licenseString.licenses.has_value() );
  EXPECT_EQ( nlohmann::json( licenseString.licenses.value() ),
             nlohmann::json( { "Unlicense" } ) );

  return true;
}

bool
test_licenseAttrs( const nix::ref<nix::EvalState> & state,
                   const std::string &              system )
{
  auto licenseAttrs
    = flox::lockFlakeInstallable( state,
                                  system,
                                  localTestFlake + "#licenseAttrs" );

  EXPECT( licenseAttrs.licenses.has_value() );
  EXPECT_EQ( nlohmann::json( licenseAttrs.licenses.value() ),
             nlohmann::json( { "Unlicense" } ) );

  return true;
}

bool
test_licenseListOfAttrs( const nix::ref<nix::EvalState> & state,
                         const std::string &              system )
{
  auto licenseListOfAttrs
    = flox::lockFlakeInstallable( state,
                                  system,
                                  localTestFlake + "#licenseListOfAttrs" );

  EXPECT( licenseListOfAttrs.licenses.has_value() );
  EXPECT_EQ( nlohmann::json( licenseListOfAttrs.licenses.value() ),
             nlohmann::json( { "Unlicense", "MIT" } ) );

  return true;
}

bool
test_licenseNoLicense( const nix::ref<nix::EvalState> & state,
                       const std::string &              system )
{
  auto noLicense
    = flox::lockFlakeInstallable( state,
                                  system,
                                  localTestFlake + "#licenseNoLicense" );

  EXPECT( ! noLicense.licenses.has_value() );

  return true;
}

bool
test_description( const nix::ref<nix::EvalState> & state,
                  const std::string &              system )
{
  auto noDescription
    = flox::lockFlakeInstallable( state, system, localTestFlake + "#hello" );

  EXPECT( ! noDescription.description.has_value() );


  auto description
    = flox::lockFlakeInstallable( state,
                                  system,
                                  localTestFlake + "#withDescription" );

  EXPECT( description.description.has_value() );
  EXPECT_EQ( description.description.value(), "A package with a description" );

  return true;
}

bool
test_names( const nix::ref<nix::EvalState> & state, const std::string & system )
{
  auto named
    = flox::lockFlakeInstallable( state, system, localTestFlake + "#names" );

  EXPECT_EQ( named.pname.value(), "hello" );
  EXPECT_EQ( named.name, "explicit-name" );

  return true;
}

bool
test_version( const nix::ref<nix::EvalState> & state,
              const std::string &              system )
{
  auto nonVersioned
    = flox::lockFlakeInstallable( state, system, localTestFlake + "#hello" );

  EXPECT( ! nonVersioned.version.has_value() );

  auto versioned = flox::lockFlakeInstallable( state,
                                               system,
                                               localTestFlake + "#versioned" );

  EXPECT_EQ( versioned.version.value(), "1.0" );

  return true;
}

bool
test_broken( const nix::ref<nix::EvalState> & state,
             const std::string &              system )
{
  auto broken
    = flox::lockFlakeInstallable( state, system, localTestFlake + "#broken" );

  // with broken = true, the package does not even evaluate
  EXPECT_EQ( broken.broken.value(), false );

  return true;
}

bool
test_unfree( const nix::ref<nix::EvalState> & state,
             const std::string &              system )
{
  auto unfree
    = flox::lockFlakeInstallable( state, system, localTestFlake + "#unfree" );

  // with unfree = true, the package does not even evaluate
  EXPECT_EQ( unfree.unfree.value(), false );

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

  RUN_TEST( attrpathUsesDefaults, state, system );
  RUN_TEST( flakerefOrigins, state, system );
  RUN_TEST( locksUrl, state, system );
  RUN_TEST( explicitOutputs, state, system );
  RUN_TEST( resolvesToDefaultPackage, state, system );
  RUN_TEST( systemAttributes, state );
  RUN_TEST( licenseString, state, system );
  RUN_TEST( licenseAttrs, state, system );
  RUN_TEST( licenseListOfAttrs, state, system );
  RUN_TEST( licenseNoLicense, state, system );
  RUN_TEST( description, state, system );
  RUN_TEST( names, state, system );
  RUN_TEST( version, state, system );
  RUN_TEST( broken, state, system );
  RUN_TEST( unfree, state, system );

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
