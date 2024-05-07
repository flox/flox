/* ========================================================================== *
 *
 *
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <iostream>

#include <nlohmann/json.hpp>

#include "flox/core/nix-state.hh"
#include "flox/core/util.hh"
#include "flox/fetchers/wrapped-nixpkgs-input.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

using namespace flox;


/* -------------------------------------------------------------------------- */

/** @brief Test a flox-nixpkgs URL can be parsed and then serialized. */
bool
test_URLRoundtrip()
{
  WrappedNixpkgsInputScheme inputScheme;
  auto                      url = "flox-nixpkgs:v0/flox/" + nixpkgsRev;
  auto input = inputScheme.inputFromURL( nix::parseURL( url ) );
  EXPECT( input.has_value() );
  EXPECT_EQ( inputScheme.toURL( *input ).to_string(), url );
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Test a flox-nixpkgs input can be created from attrs and then has the
 *        expected URL.
 **/
bool
test_inputFromAttrs()
{
  nix::fetchers::Attrs      attrs = { { "version", (uint64_t) 0 },
                                      { "type", "flox-nixpkgs" },
                                      { "owner", "NixOS" },
                                      { "rev", nixpkgsRev } };
  WrappedNixpkgsInputScheme inputScheme;
  auto                      url   = "flox-nixpkgs:v0/NixOS/" + nixpkgsRev;
  auto                      input = inputScheme.inputFromAttrs( attrs );
  EXPECT( input.has_value() );
  EXPECT_EQ( inputScheme.toURL( *input ).to_string(), url );
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Test a locked flox-nixpkgs input preserves all information in the
 *        unlocked attrs.
 **/
bool
test_lockedRepresentation( nix::ref<nix::EvalState> & state )
{
  nix::fetchers::Attrs      attrs = { { "version", (uint64_t) 0 },
                                      { "type", "flox-nixpkgs" },
                                      { "owner", "NixOS" },
                                      { "rev", nixpkgsRev } };
  WrappedNixpkgsInputScheme inputScheme;
  auto                      url   = "flox-nixpkgs:v0/NixOS/" + nixpkgsRev;
  auto                      input = inputScheme.inputFromAttrs( attrs );
  EXPECT( input.has_value() );
  auto locked = inputScheme.fetch( state->store, *input ).second;
  EXPECT( locked.toAttrs() == attrs );
  return true;
}


/* -------------------------------------------------------------------------- */

int
main()
{
  int exitCode = EXIT_SUCCESS;
  // NOLINTNEXTLINE(cppcoreguidelines-macro-usage)
#define RUN_TEST( ... ) _RUN_TEST( exitCode, __VA_ARGS__ )

  /* Initialize `nix' */
  flox::NixState nstate;
  auto           state = nstate.getState();

  RUN_TEST( URLRoundtrip );
  RUN_TEST( inputFromAttrs );
  RUN_TEST( lockedRepresentation, state );

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
