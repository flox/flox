/* ========================================================================== *
 *
 *  @file realise.cc
 *
 *  @brief Tests for `buildenv::realise` functionality.
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
    = flox::lockFlakeInstallable( state,
                                  system,
                                  "github:nixos/nixpkgs#openssl" );

  // Default outputs of openssl are `bin` and `man`
  EXPECT( defaultOutputs.outputsToInstall
          == nix::StringSet( { "bin", "man" } ) );

  auto explicitOutputs
    = flox::lockFlakeInstallable( state,
                                  system,
                                  "github:nixos/nixpkgs#openssl^dev,doc" );

  EXPECT( explicitOutputs.outputsToInstall
          == nix::StringSet( { "dev", "doc" } ) );

  auto allOutputs
    = flox::lockFlakeInstallable( state,
                                  system,
                                  "github:nixos/nixpkgs#openssl^*" );
  debugLog( "allOutputs.outputsToInstall: "
            + nlohmann::json( allOutputs.outputsToInstall ).dump() );

  EXPECT( allOutputs.outputsToInstall
          == nix::StringSet( { "bin", "dev", "out", "man", "doc" } ) );
  return true;
}


/* -------------------------------------------------------------------------- */

int
main( int argc, char * argv[] )
{
  int exitCode = EXIT_SUCCESS;
#define RUN_TEST( ... ) _RUN_TEST( exitCode, __VA_ARGS__ )

  nix::verbosity = nix::lvlDebug;
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


  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
