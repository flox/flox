/* ========================================================================== *
 *
 * @file registry.cc
 *
 * @brief Tests for `flox::Registry` interfaces.
 *
 *
 * -------------------------------------------------------------------------- */

#include <cstdlib>
#include <fstream>
#include <iostream>

#include <nlohmann/json.hpp>

#include "flox/core/util.hh"
#include "flox/registry.hh"
#include "flox/resolver/manifest.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

bool
test_FloxFlakeInputRegistry0()
{
  std::ifstream     regFile( TEST_DATA_DIR "/registry/registry0.json" );
  nlohmann::json    json = nlohmann::json::parse( regFile ).at( "registry" );
  flox::RegistryRaw regRaw;
  json.get_to( regRaw );

  flox::FloxFlakeInputFactory                 factory;
  flox::Registry<flox::FloxFlakeInputFactory> registry( regRaw, factory );
  size_t                                      count = 0;
  for ( const auto & [name, flake] : registry )
    {
      (void) flake->getFlakeRef();
      ++count;
    }

  EXPECT_EQ( count, std::size_t( 2 ) );

  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_EnvironmentManifest_getRegistryRaw0()
{
  flox::resolver::EnvironmentManifest manifest( TEST_DATA_DIR
                                                "/registry/registry0.json" );
  (void) manifest.getRegistryRaw();

  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_merge_vecs()
{
  std::vector<std::string> highPriority = { "a", "b", "c" };
  std::vector<std::string> lowPriority  = { "a", "d", "e" };
  std::vector<std::string> expected     = { "a", "b", "c", "d", "e" };
  auto merged = flox::merge_vectors( lowPriority, highPriority );
  EXPECT( merged == expected );
  return true;
}

/* -------------------------------------------------------------------------- */

bool
test_EnvironmentManifest_badPath0()
{
  /* Try loading the registry without setting the path. */
  try
    {
      flox::resolver::EnvironmentManifest manifest( "" );
      (void) manifest.getRegistryRaw();
      return false;
    }
  catch ( flox::FloxException & )
    {
      return true;
    }
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Ensure we throw an error if a manifest contains indirect flake
 *        references in its registry.
 *
 * This should "fail early" when processing the `getRegistryRaw()` rather than
 * waiting for `getLockedRegistry()`
 * ( which invokes the `Registry<T>()` contstructor ) to catch the error.
 */
bool
test_EnvironmentManifest_NoIndirectRefs0()
{
  try
    {
      flox::resolver::EnvironmentManifest manifest(
        TEST_DATA_DIR "/registry/registry1.json" );
      (void) manifest.getRegistryRaw();
      return false;
    }
  catch ( flox::FloxException & )
    {
      return true;
    }
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

  RUN_TEST( FloxFlakeInputRegistry0 );

  RUN_TEST( EnvironmentManifest_getRegistryRaw0 );
  RUN_TEST( EnvironmentManifest_badPath0 );
  RUN_TEST( EnvironmentManifest_NoIndirectRefs0 );
  RUN_TEST( merge_vecs );

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
