/* ========================================================================== *
 *
 *
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <iostream>

#include <nix/attrs.hh>
#include <nlohmann/json.hpp>

#include "flox/core/util.hh"
#include "flox/registry/floxpkgs.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

/**
 * Scraping should be cross platform, so even though this is hardcoded, it
 * should work on other systems.
 */
const flox::System _system = "x86_64-linux";

nlohmann::json floxpkgsAttrsJson = {
  { "owner", "NixOS" },
  { "repo", "nixpkgs" },
  { "rev", nixpkgsRev },
  { "type", flox::FLOX_FLAKE_TYPE },
};
nix::fetchers::Attrs floxpkgsAttrs
  = nix::fetchers::jsonToAttrs( floxpkgsAttrsJson );
std::string floxpkgsURL
  = flox::FLOX_FLAKE_TYPE + ":NixOS/nixpkgs/" + nixpkgsRev;
nix::ParsedURL floxpkgsParsedURL = nix::parseURL( floxpkgsURL );


/* -------------------------------------------------------------------------- */

bool
test_constructsInputFromURL()
{
  auto input = nix::fetchers::Input::fromURL( floxpkgsParsedURL );
  EXPECT( input.scheme != nullptr );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_constructsInputFromAttrs()
{
  auto input = nix::fetchers::Input::fromAttrs(
    nix::fetchers::jsonToAttrs( floxpkgsAttrsJson ) );
  EXPECT( input.scheme != nullptr );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_canConvertToURL()
{
  flox::FloxFlakeScheme scheme;
  auto                  input = scheme.inputFromURL( floxpkgsParsedURL );
  EXPECT( input.has_value() );
  auto url = ( *input ).toURLString();
  EXPECT_EQ( url, floxpkgsURL );
  return true;
}

/* -------------------------------------------------------------------------- */

bool
test_ignoresWrongInputType()
{
  flox::FloxFlakeScheme scheme;
  auto                  url        = "github:NixOS/nixpkgs/release-23.05";
  auto                  parsed     = nix::parseURL( url );
  auto                  maybeInput = scheme.inputFromURL( parsed );
  EXPECT( maybeInput == std::nullopt );
  return true;
}

/* -------------------------------------------------------------------------- */


int
main()
{

  int exitCode = EXIT_SUCCESS;
  // NOLINTNEXTLINE(cppcoreguidelines-macro-usage)
#define RUN_TEST( ... ) _RUN_TEST( exitCode, __VA_ARGS__ )

  RUN_TEST( constructsInputFromURL );
  RUN_TEST( constructsInputFromAttrs );
  RUN_TEST( ignoresWrongInputType );
  RUN_TEST( canConvertToURL );
  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
