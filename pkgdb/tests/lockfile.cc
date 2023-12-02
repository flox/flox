/* ========================================================================== *
 *
 *  FIXME: real hashes
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <iostream>

#include <nlohmann/json.hpp>

#include "flox/resolver/lockfile.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

using namespace nlohmann::literals;


/* -------------------------------------------------------------------------- */

bool
test_LockedInputRawFromJSON0()
{
  using namespace flox::resolver;
  nlohmann::json json = {
    { "fingerprint", nixpkgsFingerprintStr },
    { "url", nixpkgsRef },
    { "attrs",
      { { "owner", "NixOS" }, { "repo", "nixpkgs" }, { "rev", nixpkgsRev } } }
  };
  LockedInputRaw raw( json );
  return true;
}


/* -------------------------------------------------------------------------- */

bool
test_LockedPackageRawFromJSON0()
{
  using namespace flox::resolver;
  nlohmann::json json
    = { { "input",
          { { "fingerprint", nixpkgsFingerprintStr },
            { "url", nixpkgsRef },
            { "attrs",
              { { "owner", "NixOS" },
                { "repo", "nixpkgs" },
                { "rev", nixpkgsRev } } } } },
        { "attr-path", { "legacyPackages", "x86_64-linux", "hello" } },
        { "priority", 5 },
        { "info", {} } };
  LockedPackageRaw raw( json );
  return true;
}


/* -------------------------------------------------------------------------- */

int
main()
{
  int exitCode = EXIT_SUCCESS;
  // NOLINTNEXTLINE(cppcoreguidelines-macro-usage)
#define RUN_TEST( ... ) _RUN_TEST( exitCode, __VA_ARGS__ )

  RUN_TEST( LockedInputRawFromJSON0 );

  RUN_TEST( LockedPackageRawFromJSON0 );

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
