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
#include "test.hh"


/* -------------------------------------------------------------------------- */


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

  RUN_TEST( merge_vecs );


  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
