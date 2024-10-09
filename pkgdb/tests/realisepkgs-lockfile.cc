
/* ========================================================================== *
 *
 *  FIXME: real hashes
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <iostream>

#include <nlohmann/json.hpp>

#include "flox/realisepkgs/realisepkgs-lockfile.hh"
#include "flox/core/util.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

using namespace nlohmann::literals;
using namespace flox::realisepkgs;

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
      },
      "myflake": {
        "flake": "github:NixOS/nixpkgs#hello",
        "priority": 6
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
      "locked_url": "https://github.com/flox/nixpkgs?rev=9a333eaa80901efe01df07eade2c16d183761fa3",
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
    },
    {
      "install_id": "myflake",
      "locked-url": "github:NixOS/nixpkgs/c35032a3c98e9bbdd8eef7337d9de2cb5f174d99",
      "flake-description": "A collection of packages for the Nix package manager",
      "locked-flake-attr-path": "legacyPackages.x86_64-linux.hello",
      "derivation": "/nix/store/fqs92lzychkm6p37j7fnj4d65nq9fzla-hello-2.12.1.drv",
      "outputs": {
        "out": "/nix/store/kwmqk7ygvhypxadsdaai27gl6qfxv7za-hello-2.12.1"
      },
      "output-names": [
        "out"
      ],
      "outputs-to-install": [
        "out"
      ],
      "package-system": "x86_64-linux",
      "system": "x86_64-linux",
      "name": "hello-2.12.1",
      "pname": "hello",
      "version": "2.12.1",
      "description": "Program that produces a familiar, friendly greeting",
      "licenses": [
        "GPL-3.0-or-later"
      ],
      "broken": false,
      "unfree": false,
      "priority": 6
    }
  ]
} )";


/* -------------------------------------------------------------------------- */

bool
test_LockfileFromV1()
{
  nlohmann::json   json     = flox::parseOrReadJSONObject( lockfileContentV1 );
  RealisepkgsLockfile lockfile = RealisepkgsLockfile();
  lockfile.load_from_content( json );
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

  EXPECT( lockfile.packages.size() == 2 );
  auto pkg = lockfile.packages[0];
  EXPECT_EQ( pkg.installId, "mycowsay" );

  // The attr path is pre-pended for compatibility reasons
  flox::AttrPath attrPath = { "legacyPackages", "x86_64-linux", "cowsay" };
  EXPECT( pkg.attrPath == attrPath );

  EXPECT_EQ( pkg.input.url,
             "flox-nixpkgs:v0/flox/9a333eaa80901efe01df07eade2c16d183761fa3" );
  EXPECT_EQ( pkg.input.attrs["version"], 0 );
  EXPECT_EQ( pkg.input.attrs["rev"],
             "9a333eaa80901efe01df07eade2c16d183761fa3" );
  EXPECT_EQ( pkg.input.attrs["owner"], "flox" );
  EXPECT_EQ( pkg.input.attrs["type"], "flox-nixpkgs" );

  auto flakepkg = lockfile.packages[1];
  EXPECT_EQ( flakepkg.priority, (uint64_t) 6 );

  return true;
}

/* -------------------------------------------------------------------------- */

int
main()
{
  int exitCode = EXIT_SUCCESS;
  // NOLINTNEXTLINE(cppcoreguidelines-macro-usage)
#define RUN_TEST( ... ) _RUN_TEST( exitCode, __VA_ARGS__ )

  RUN_TEST( LockfileFromV1 );

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
