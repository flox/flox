/* ========================================================================== *
 *
 *  FIXME: real hashes
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <iostream>

#include <nlohmann/json.hpp>

#include "flox/core/util.hh"
#include "flox/resolver/lockfile.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

using namespace nlohmann::literals;


static const std::string lockfileContentV1 = R"( {
  "lockfile-version": 1,
  "manifest": {
    "hook": {
      "on-activate": "my_onactivate"
    },
    "install": {
      "hello": {
        "optional": false,
        "package-group": "group",
        "pkg-path": "hello",
        "priority": null,
        "systems": null,
        "version": null
      }
    },
    "options": {
      "allow": {
        "broken": null,
        "licenses": [],
        "unfree": null
      },
      "semver": {
        "prefer-pre-releases": null
      },
      "systems": [
        "system"
      ]
    },
    "profile": {
      "bash": "profile.bash",
      "common": "profile.common",
      "zsh": "profile.zsh"
    },
    "vars": {"TEST": "VAR"},
    "version": 1
  },
  "packages": [
    {
      "install_id": "mycowsay",
      "group": "mygroupname",
      "priority": 1,
      "optional": false,
      "attr_path": "cowsay",
      "broken": false,
      "derivation": "derivation",
      "description": "description",
      "license": "license",
      "locked_url": "github:NixOS/nixpkgs/9a333eaa80901efe01df07eade2c16d183761fa3",
      "name": "hello",
      "outputs": {
        "name": "store_path"
      },
      "outputs_to_install": [
        "name"
      ],
      "pname": "pname",
      "rev": "rev",
      "rev_count": 1,
      "rev_date": "2021-08-31T00:00:00Z",
      "scrape_date": "2021-08-31T00:00:00Z",
      "stabilities": [
        "stability"
      ],
      "system": "x86_64-linux",
      "unfree": false,
      "version": "version"
    }
  ]
} )";


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

bool
test_LockfileFromV1()
{
  using namespace flox::resolver;
  nlohmann::json json     = flox::parseOrReadJSONObject( lockfileContentV1 );
  LockfileRaw    lockfile = LockfileRaw();
  lockfile.load_from_content( json );
  EXPECT( lockfile.lockfileVersion == 1 );
  EXPECT( lockfile.manifest.hook.has_value() );
  EXPECT_EQ( lockfile.manifest.hook.value().onActivate.value_or( "" ),
             "my_onactivate" );

  EXPECT( lockfile.manifest.profile.has_value() );
  EXPECT_EQ( lockfile.manifest.profile.value().common.value(),
             "profile.common" );
  EXPECT_EQ( lockfile.manifest.profile.value().bash.value(), "profile.bash" );
  EXPECT_EQ( lockfile.manifest.profile.value().zsh.value(), "profile.zsh" );

  EXPECT( lockfile.manifest.vars.has_value() );
  EXPECT( lockfile.manifest.vars.value().size() == 1 );
  EXPECT_EQ( lockfile.manifest.vars.value()["TEST"], "VAR" );

  auto packages = lockfile.packages.at( "x86_64-linux" );
  EXPECT( packages.size() == 1 );

  auto pkg = packages["mycowsay"];
  // The attr path is pre-pended for compatibility reasons
  flox::AttrPath attrPath = { "legacyPackages", "x86_64-linux", "cowsay" };
  EXPECT( pkg.value().attrPath == attrPath );

  EXPECT_EQ( pkg.value().input.url,
             "github:NixOS/nixpkgs/9a333eaa80901efe01df07eade2c16d183761fa3" );
  EXPECT_EQ( pkg.value().input.attrs["rev"],
             "9a333eaa80901efe01df07eade2c16d183761fa3" );
  // These are assumed from v1
  EXPECT_EQ( pkg.value().input.attrs["owner"], "NixOS" );
  EXPECT_EQ( pkg.value().input.attrs["type"], "github" );
  EXPECT_EQ( pkg.value().input.attrs["repo"], "nixpkgs" );
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

  RUN_TEST( LockfileFromV1 );

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
