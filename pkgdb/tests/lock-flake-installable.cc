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


/* -------------------------------------------------------------------------- */

bool
test_attrpathUsesDefaults( const nix::ref<nix::EvalState> & state,
                           const std::string &              system )
{
  auto lockedExplicit = flox::lockFlakeInstallable(
    state,
    system,
    "github:nixos/nixpkgs#legacyPackages." + system + ".hello" );

  auto lockedImplicit
    = flox::lockFlakeInstallable( state, system, "github:nixos/nixpkgs#hello" );

  EXPECT_EQ( nlohmann::json( lockedExplicit ),
             nlohmann::json( lockedImplicit ) );

  EXPECT_EQ( lockedImplicit.lockedAttrPath,
             "legacyPackages." + system + ".hello" );

  return true;
}

/**
 * @brief Test that the flake origin is correctly parsed from the flake
 */
bool
test_flakerefOrigins( const nix::ref<nix::EvalState> & state,
                      const std::string &              system )
{
  auto githubScheme = flox::lockFlakeInstallable( state,
                                                  system,
                                                  "github:nixos/nixpkgs/"
                                                    + nixpkgsRev + "#hello" );
  auto gitHttpsScheme
    = flox::lockFlakeInstallable( state, system, nixpkgsRef + "#hello" );


  return true;
}

/**
 * @brief Test that the flake origin is correctly parsed from the flake
 */
bool
test_locksUrl( const nix::ref<nix::EvalState> & state,
               const std::string &              system )
{
  auto unlockedUrl = "github:nixos/nixpkgs#hello";
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
    = flox::lockFlakeInstallable( state, system, "github:nixos/nixpkgs#rustc" );

  // Default outputs of openssl are `bin` and `man`
  EXPECT_EQ( nlohmann::json( defaultOutputs.outputsToInstall ),
             nlohmann::json( nix::StringSet( { "out", "man" } ) ) );

  auto explicitOutputs
    = flox::lockFlakeInstallable( state,
                                  system,
                                  "github:nixos/nixpkgs#rustc^man,doc" );

  EXPECT_EQ( nlohmann::json( explicitOutputs.outputsToInstall ),
             nlohmann::json( nix::StringSet( { "man", "doc" } ) ) );

  auto allOutputs
    = flox::lockFlakeInstallable( state,
                                  system,
                                  "github:nixos/nixpkgs#rustc^*" );


  EXPECT_EQ( nlohmann::json( allOutputs.outputsToInstall ),
             nlohmann::json( nix::StringSet( { "out", "man", "doc" } ) ) );

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
    = flox::lockFlakeInstallable( state, system, "github:flox/flox" );

  auto explicitPackage = flox::lockFlakeInstallable(
    state,
    system,
    "github:flox/flox#packages." + system + ".default" );

  EXPECT_EQ( nlohmann::json( defaultPackage ),
             nlohmann::json( explicitPackage ) );
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


  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
