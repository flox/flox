/* ========================================================================== *
 *
 *
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <iostream>

#include <nlohmann/json.hpp>

#include "flox/core/util.hh"
#include "flox/resolver/environment.hh"
#include "flox/resolver/manifest.hh"
#include "test.hh"


/* -------------------------------------------------------------------------- */

using namespace flox;
using namespace flox::resolver;
using namespace nlohmann::literals;


/* -------------------------------------------------------------------------- */

class TestEnvironment : public Environment
{
public:

  using Environment::Environment;
  using Environment::getGroupInput;
  using Environment::groupIsLocked;
};

/**
 * Scraping should be cross platform, so even though this is hardcoded, it
 * should work on other systems.
 */
const System _system = "x86_64-linux";


/* -------------------------------------------------------------------------- */

nlohmann::json registryWithNixpkgsJSON {
  { "inputs",
    { { "nixpkgs",
        { { "from",
            { { "type", "flox-nixpkgs" },
              { "owner", "NixOS" },
              { "repo", "nixpkgs" },
              { "rev", nixpkgsRev } } },
          { "subtrees", { "legacyPackages" } } } } } }
};
RegistryRaw registryWithNixpkgs( registryWithNixpkgsJSON );


/* -------------------------------------------------------------------------- */

nlohmann::json inputWithNixpkgsJSON {
  "input",
  { { "fingerprint", nixpkgsFingerprintStr },
    { "url", nixpkgsRef },
    { "attrs",
      { { "owner", "NixOS" },
        { "repo", "nixpkgs" },
        { "rev", nixpkgsRev },
        { "type", "github" },
        { "lastModified", 1704300003 },
        { "narHash",
          "sha256-FRC/OlLVvKkrdm+RtrODQPufD0vVZYA0hpH9RPaHmp4=" } } } }
};


/* -------------------------------------------------------------------------- */

nlohmann::json mockInputJSON {
  "input",
  { { "fingerprint", nixpkgsFingerprintStr },
    { "url", nixpkgsRef },
    { "attrs",
      { { "owner", "owner" },
        { "repo", "repo" },
        { "rev", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" },
        { "type", "github" },
        { "lastModified", 1704300003 },
        { "narHash",
          "sha256-FRC/OlLVvKkrdm+RtrODQPufD0vVZYA0hpH9RPaHmp4=" } } } }
};


/* -------------------------------------------------------------------------- */

nlohmann::json helloLockedJSON {
  inputWithNixpkgsJSON,
  { "attr-path", { "legacyPackages", _system, "hello" } },
  { "priority", 5 },
  { "info",
    {
      { "broken", false },
      { "license", "GPL-3.0-or-later" },
      { "pname", "hello" },
      { "unfree", false },
      { "version", "2.12.1" },
      { "description",
        "A program that produces a familiar, friendly greeting" },
    } }
};
LockedPackageRaw helloLocked( helloLockedJSON );


/* -------------------------------------------------------------------------- */

/** Change a few fields from what we'd get if actual resultion was performed. */
nlohmann::json mockHelloLockedJSON {
  mockInputJSON,
  { "attr-path", { "mock", "hello" } },
  { "priority", 5 },
  { "info",
    {
      { "broken", false },
      { "license", "GPL-3.0-or-later" },
      { "pname", "hello" },
      { "unfree", false },
      { "version", "2.12.1" },
      { "description",
        "A program that produces a familiar, friendly greeting" },
    } }
};
LockedPackageRaw mockHelloLocked( mockHelloLockedJSON );


/* -------------------------------------------------------------------------- */

nlohmann::json curlLockedJSON {
  inputWithNixpkgsJSON,
  { "attr-path", { "legacyPackages", _system, "curl" } },
  { "priority", 5 },
  { "info",
    {
      { "broken", false },
      { "license", "curl" },
      { "pname", "curl" },
      { "unfree", false },
      { "version", curlVersion },
      { "description",
        "A command line tool for transferring files with URL syntax" },
    } }
};
LockedPackageRaw curlLocked( curlLockedJSON );


/* -------------------------------------------------------------------------- */

/** Change a few fields from what we'd get if actual resultion was performed.
 */
nlohmann::json mockCurlLockedJSON {
  mockInputJSON,
  { "attr-path", { "mock", "curl" } },
  { "priority", 5 },
  { "info",
    {
      { "broken", false },
      { "license", "GPL-3.0-or-later" },
      { "pname", "curl" },
      { "unfree", false },
      { "version", "2.12.1" },
      { "description",
        "A command line tool for transferring files with URL syntax" },
    } }
};
LockedPackageRaw mockCurlLocked( mockCurlLockedJSON );


/* -------------------------------------------------------------------------- */

bool
equalLockfileRaw( const LockfileRaw & first, const LockfileRaw & second )
{
  for ( const auto & [system, firstSystemPackages] : first.packages )
    {
      EXPECT( second.packages.contains( system ) );
      const SystemPackages & secondSystemPackages
        = second.packages.at( system );
      for ( const auto & [installID, lockedPackageRaw] : firstSystemPackages )
        {
          EXPECT( secondSystemPackages.contains( installID ) );
          const std::optional<LockedPackageRaw> & secondLockedPackageRaw
            = secondSystemPackages.at( installID );
          EXPECT( lockedPackageRaw.has_value()
                  == secondLockedPackageRaw.has_value() );
          if ( lockedPackageRaw.has_value()
               && secondLockedPackageRaw.has_value() )
            {
              if ( ( *lockedPackageRaw ) != ( *secondLockedPackageRaw ) )
                {
                  return false;
                };
            }
        }
    }
  // TODO check:
  // ManifestRaw manifest;
  // RegistryRaw                                registry;
  // unsigned                                   lockfileVersion = 0;
  return true;
}


/* -------------------------------------------------------------------------- */

bool
equalLockfile( const Lockfile & first, const Lockfile & second )
{
  return equalLockfileRaw( first.getLockfileRaw(), second.getLockfileRaw() );
  // TODO check:
  // std::optional<std::filesystem::path> lockfilePath;
  // Manifest                             manifest;
  // RegistryRaw packagesRegistryRaw;
}


/* -------------------------------------------------------------------------- */

/** @brief Test unmodified manifest descriptor stays locked. */
bool
test_groupIsLocked0()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello", std::nullopt } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  EnvironmentManifest manifest( manifestRaw );

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system, { { "hello", helloLocked } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* All groups should be locked. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( environment.groupIsLocked( name, group, lockfile, _system ) );
    }

  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Test that explicitly requiring the locked system doesn't unlock
 *        the group.
 */
bool
test_groupIsLocked1()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello",
                                     ManifestDescriptorRaw(
                              nlohmann::json( { { "path", "hello" } } ) ) } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system, { { "hello", helloLocked } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* Explicitly require the already locked system. */
  ManifestRaw    modifiedManifestRaw( manifestRaw );
  nlohmann::json helloJSON = { { "systems", { _system } } };
  modifiedManifestRaw.install->at( "hello" )
    = ManifestDescriptorRaw( helloJSON );
  EnvironmentManifest manifest( modifiedManifestRaw );

  /* All groups should be locked. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( environment.groupIsLocked( name, group, lockfile, _system ) );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Test disabling the locked system unlocks the group. */
bool
test_groupIsLocked2()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello", std::nullopt } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system, { { "hello", helloLocked } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* Don't support the current system. */
  ManifestRaw    modifiedManifestRaw( manifestRaw );
  nlohmann::json helloJSON = { { "systems", nlohmann::json::array() } };
  modifiedManifestRaw.install->at( "hello" )
    = ManifestDescriptorRaw( helloJSON );
  EnvironmentManifest manifest( modifiedManifestRaw );

  /* All groups should be unlocked. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( ! environment.groupIsLocked( name, group, lockfile, _system ) );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Test moving a package to a different group unlocks it. */
bool
test_groupIsLocked3()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello", std::nullopt } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system, { { "hello", helloLocked } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* Move hello to the `red' group. */
  ManifestRaw    modifiedManifestRaw( manifestRaw );
  nlohmann::json helloJSON = { { "package-group", "red" } };
  modifiedManifestRaw.install->at( "hello" )
    = ManifestDescriptorRaw( helloJSON );
  EnvironmentManifest manifest( modifiedManifestRaw );

  /* All groups should be unlocked. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( ! environment.groupIsLocked( name, group, lockfile, _system ) );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Test adding a package to the default group unlocks it. */
bool
test_groupIsLocked4()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello", std::nullopt } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system, { { "hello", helloLocked } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* Add curl to the manifest (but not the lockfile) */
  ManifestRaw modifiedManifestRaw( manifestRaw );
  ( *modifiedManifestRaw.install )["curl"] = std::nullopt;
  EnvironmentManifest manifest( modifiedManifestRaw );

  /* All groups should be unlocked. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( ! environment.groupIsLocked( name, group, lockfile, _system ) );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief Test adding a package to a different group doesn't unlock the
 *        default group.
 */
bool
test_groupIsLocked5()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello", std::nullopt } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system, { { "hello", helloLocked } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* Add curl to a separate group in the manifest, but not the lockfile */
  ManifestRaw    modifiedManifestRaw( manifestRaw );
  nlohmann::json curlJSON                  = { { "package-group", "blue" } };
  ( *modifiedManifestRaw.install )["curl"] = ManifestDescriptorRaw( curlJSON );
  EnvironmentManifest manifest( manifestRaw );

  /* The group with hello should stay locked, but curl should be unlocked. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  auto            groups = manifest.getGroupedDescriptors();
  for ( const auto & [name, group] : groups )
    {
      if ( group.contains( "hello" ) )
        {
          EXPECT( environment.groupIsLocked( name, group, lockfile, _system ) );
        }
      else
        {
          EXPECT(
            ! environment.groupIsLocked( name, group, lockfile, _system ) );
        }
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Test that two separate groups both stay locked. */
bool
test_groupIsLocked6()
{
  /* Create manifest with hello and curl */
  ManifestRaw    manifestRaw;
  nlohmann::json curlJSON = { { "package-group", "blue" } };
  manifestRaw.install     = { { "hello", std::nullopt }, { "curl", curlJSON } };
  manifestRaw.options     = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;
  EnvironmentManifest manifest( manifestRaw );

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw lockfileRaw;
  lockfileRaw.packages
    = { { _system, { { "hello", helloLocked }, { "curl", curlLocked } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* All groups should be locked. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( environment.groupIsLocked( name, group, lockfile, _system ) );
    }

  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief Test upgrades correctly control locking. */
bool
test_groupIsLocked_upgrades()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello", std::nullopt } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  EnvironmentManifest manifest( manifestRaw );

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system, { { "hello", helloLocked } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* Reuse lock when upgrades = false. */
  TestEnvironment environment( std::nullopt, manifest, lockfile, false );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( environment.groupIsLocked( name, group, lockfile, _system ) );
    }

  /* Re-lock when upgrades = true. */
  environment = TestEnvironment( std::nullopt, manifest, lockfile, true );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( ! environment.groupIsLocked( name, group, lockfile, _system ) );
    }

  /* Reuse lock when TOPLEVEL_GROUP_NAME not in upgrades list. */
  environment = TestEnvironment( std::nullopt,
                                 manifest,
                                 lockfile,
                                 std::vector<GroupName> {} );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( environment.groupIsLocked( name, group, lockfile, _system ) );
    }

  /* Reuse lock when `hello' in upgrades list. */
  environment = TestEnvironment( std::nullopt,
                                 manifest,
                                 lockfile,
                                 std::vector<GroupName> { "hello" } );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( environment.groupIsLocked( name, group, lockfile, _system ) );
    }

  /* Re-lock when TOPLEVEL_GROUP_NAME is in upgrades list. */
  environment
    = TestEnvironment( std::nullopt,
                       manifest,
                       lockfile,
                       std::vector<GroupName> { TOPLEVEL_GROUP_NAME } );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      EXPECT( ! environment.groupIsLocked( name, group, lockfile, _system ) );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief `getGroupInput` returns the locked input even if the group
 *        name changes.
 */
bool
test_getGroupInput0()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello",
                                     ManifestDescriptorRaw(
                              nlohmann::json( { { "path", "hello" } } ) ) } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system, { { "hello", mockHelloLocked } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* Name the group hello is in. */
  ManifestRaw    modifiedManifestRaw( manifestRaw );
  nlohmann::json helloJSON = { { "package-group", "blue" } };
  modifiedManifestRaw.install->at( "hello" )
    = ManifestDescriptorRaw( helloJSON );
  EnvironmentManifest manifest( modifiedManifestRaw );

  /* The locked input is returned by `getGroupInput`. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      auto input = environment.getGroupInput( group, lockfile, _system );
      EXPECT( input.has_value() );
      EXPECT_EQ( *input, mockHelloLocked.input );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief `getGroupInput` respects the lock of a package that used to be in a
 *        group over a package that was just added.
 */
bool
test_getGroupInput1()
{
  /* Create manifest with hello and curl in separate group */
  ManifestRaw           manifestRaw;
  nlohmann::json        curlJSON = { { "package-group", "blue" } };
  ManifestDescriptorRaw curl( curlJSON );
  manifestRaw.install = { { "hello", std::nullopt }, { "curl", curl } };
  manifestRaw.options = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  /* Create expected lockfile, reusing manifestRaw, and ensuring curl's input is
   * different than hello's. */
  LockedPackageRaw mockCurlLockedModified( mockCurlLocked );
  mockCurlLockedModified.input.url = "not the same as hello's input";
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system,
                             { { "hello", mockHelloLocked },
                               { "curl", mockCurlLockedModified } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* Move hello to the same group as curl. */
  ManifestRaw    modifiedManifestRaw( manifestRaw );
  nlohmann::json helloJSON = { { "package-group", "blue" } };
  modifiedManifestRaw.install->at( "hello" )
    = ManifestDescriptorRaw( helloJSON );
  EnvironmentManifest manifest( modifiedManifestRaw );

  /* The locked input is returned by `getGroupInput`. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      auto input = environment.getGroupInput( group, lockfile, _system );
      EXPECT( input.has_value() );
      // TODO this prints "Expectation failed" which should probably be silenced
      EXPECT( ( *input ) != mockHelloLocked.input );
      EXPECT_EQ( *input, mockCurlLockedModified.input );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief `getGroupInput` uses a locked input when two groups are combined into
 *        a group with a new name.
 */
bool
test_getGroupInput2()
{
  /* Create manifest with hello and curl in separate group */
  ManifestRaw           manifestRaw;
  nlohmann::json        curlJSON = { { "package-group", "blue" } };
  ManifestDescriptorRaw curl( curlJSON );
  manifestRaw.install = { { "hello", std::nullopt }, { "curl", curl } };
  manifestRaw.options = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  /* Create expected lockfile, reusing manifestRaw, and ensuring curl's input is
   * different than hello's. */
  LockedPackageRaw mockCurlLockedModified( mockCurlLocked );
  mockCurlLockedModified.input.url = "not the same as hello's input";
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system,
                             { { "hello", mockHelloLocked },
                               { "curl", mockCurlLockedModified } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* Move hello and curl to a new group. */
  ManifestRaw    modifiedManifestRaw( manifestRaw );
  nlohmann::json helloJSON = { { "package-group", "new-blue" } };
  modifiedManifestRaw.install->at( "hello" )
    = ManifestDescriptorRaw( helloJSON );
  nlohmann::json modifiedCurlJSON = { { "package-group", "new-blue" } };
  modifiedManifestRaw.install->at( "curl" )
    = ManifestDescriptorRaw( modifiedCurlJSON );
  EnvironmentManifest manifest( modifiedManifestRaw );

  /* The locked input of one of the packages is returned. At this point, we
   * don't care which. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      auto input = environment.getGroupInput( group, lockfile, _system );
      EXPECT( input.has_value() );
      EXPECT( ( ( *input ) == mockHelloLocked.input )
              || ( ( *input ) == mockCurlLockedModified.input ) );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief `getGroupInput` does not use a locked input if the package
 *        has changed.
 */
bool
test_getGroupInput3()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello", std::nullopt } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw lockfileRaw;
  lockfileRaw.packages = { { _system, { { "hello", mockHelloLocked } } } };
  lockfileRaw.manifest = manifestRaw;

  Lockfile lockfile( lockfileRaw );

  /* Add a version for hello. */
  ManifestRaw    modifiedManifestRaw( manifestRaw );
  nlohmann::json helloJSON = { { "version", "2.12" } };
  modifiedManifestRaw.install->at( "hello" )
    = ManifestDescriptorRaw( helloJSON );
  EnvironmentManifest manifest( modifiedManifestRaw );

  /* The old locked input is *not* used. */
  TestEnvironment environment( std::nullopt, manifest, lockfile );
  for ( const auto & [name, group] : manifest.getGroupedDescriptors() )
    {
      auto input = environment.getGroupInput( group, lockfile, _system );
      EXPECT( ! input.has_value() );
    }
  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief createLockfile creates a lock when there is no existing lockfile. */
bool
test_createLockfile_new()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello", std::nullopt } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  EnvironmentManifest manifest( manifestRaw );

  /* Create expected lockfile, reusing manifestRaw */
  LockfileRaw expectedLockfileRaw;
  expectedLockfileRaw.packages = { { _system, { { "hello", helloLocked } } } };
  expectedLockfileRaw.manifest = manifestRaw;

  Lockfile expectedLockfile( expectedLockfileRaw );

  /* Test locking manifest creates expectedLockfile */
  Environment environment( std::nullopt, manifest, std::nullopt );
  Lockfile    actualLockfile = environment.createLockfile();
  EXPECT( equalLockfile( actualLockfile, expectedLockfile ) );

  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief `createLockfile()` reuses existing lockfile entry. */
bool
test_createLockfile_existing()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "hello", std::nullopt } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  EnvironmentManifest manifest( manifestRaw );

  /* Create existing/expected lockfile, reusing manifestRaw */
  LockfileRaw expectedLockfileRaw;
  expectedLockfileRaw.packages
    = { { _system, { { "hello", mockHelloLocked } } } };
  expectedLockfileRaw.manifest = manifestRaw;

  Lockfile expectedLockfile( expectedLockfileRaw );

  /* Test locking manifest reuses existing lockfile */
  Environment    environment( std::nullopt, manifest, expectedLockfile );
  Lockfile       actualLockfile = environment.createLockfile();
  nlohmann::json actualJson;
  to_json( actualJson, actualLockfile.getLockfileRaw() );
  std::cerr << "actual: " << actualJson << std::endl;
  nlohmann::json expectedJson;
  to_json( expectedJson, expectedLockfile.getLockfileRaw() );
  std::cerr << "expected: " << expectedJson << std::endl;
  EXPECT( equalLockfile( actualLockfile, expectedLockfile ) );

  return true;
}


/* -------------------------------------------------------------------------- */

/**
 * @brief `createLockfile()` both reuses existing lockfile entry and locks
 *        unlocked packages.
 */
bool
test_createLockfile_both()
{
  /* Create manifest with hello and curl in separate group */
  ManifestRaw           manifestRaw;
  nlohmann::json        curlJSON = { { "package-group", "blue" } };
  ManifestDescriptorRaw curl( curlJSON );
  manifestRaw.install = { { "hello", std::nullopt }, { "curl", curl } };
  manifestRaw.options = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  EnvironmentManifest manifest( manifestRaw );

  /* Create existing lockfile with hello but not curl. We have to create another
   * manifest with hello but not curl. */
  ManifestRaw existingManifestRaw;
  existingManifestRaw.install = {
    { "hello", std::nullopt },
  };
  existingManifestRaw.options          = Options {};
  existingManifestRaw.options->systems = { _system };
  existingManifestRaw.registry         = registryWithNixpkgs;

  EnvironmentManifest existingManifest( existingManifestRaw );

  LockfileRaw existingLockfileRaw;
  existingLockfileRaw.packages
    = { { _system, { { "hello", mockHelloLocked } } } };
  existingLockfileRaw.manifest = existingManifestRaw;

  Lockfile existingLockfile( existingLockfileRaw );

  /* Create expected lockfile with both hello and curl, reusing manifestRaw */
  LockfileRaw expectedLockfileRaw;
  expectedLockfileRaw.packages
    = { { _system, { { "hello", mockHelloLocked }, { "curl", curlLocked } } } };
  expectedLockfileRaw.manifest = manifestRaw;

  Lockfile expectedLockfile( expectedLockfileRaw );

  /* Test the lock for hello gets used, but curl gets locked */
  Environment environment( std::nullopt, manifest, existingLockfile );
  Lockfile    actualLockfile = environment.createLockfile();
  EXPECT( equalLockfile( actualLockfile, expectedLockfile ) );

  return true;
}


/* -------------------------------------------------------------------------- */

/** @brief createLockfile gives a helpful error message when a package can't be
 * resolved. */
bool
test_createLockfile_error()
{
  /* Create manifest with hello */
  ManifestRaw manifestRaw;
  manifestRaw.install          = { { "not-a-package", std::nullopt } };
  manifestRaw.options          = Options {};
  manifestRaw.options->systems = { _system };
  manifestRaw.registry         = registryWithNixpkgs;

  EnvironmentManifest manifest( manifestRaw );

  /* Test locking manifest creates expectedLockfile */
  Environment environment( std::nullopt, manifest, std::nullopt );
  try
    {
      Lockfile actualLockfile = environment.createLockfile();
      return false;
    }
  catch ( const flox::resolver::ResolutionFailureException & err )
    {
      EXPECT_EQ( err.getErrorCode(), flox::EC_RESOLUTION_FAILURE );
      return true;
    }
  catch ( const flox::FloxException & err )
    {
      std::cerr << err.what() << std::endl;
      return false;
    }
  return false;
}


/* -------------------------------------------------------------------------- */

int
main()
{

  int exitCode = EXIT_SUCCESS;
  // NOLINTNEXTLINE(cppcoreguidelines-macro-usage)
#define RUN_TEST( ... ) _RUN_TEST( exitCode, __VA_ARGS__ )

  // RUN_TEST( groupIsLocked0 );
  // RUN_TEST( groupIsLocked1 );
  // RUN_TEST( groupIsLocked2 );
  // RUN_TEST( groupIsLocked3 );
  // RUN_TEST( groupIsLocked4 );
  // RUN_TEST( groupIsLocked5 );
  // RUN_TEST( groupIsLocked6 );
  // RUN_TEST( groupIsLocked_upgrades );

  // RUN_TEST( getGroupInput0 );
  // RUN_TEST( getGroupInput1 );
  // RUN_TEST( getGroupInput2 );
  // RUN_TEST( getGroupInput3 );

  // RUN_TEST( createLockfile_new );
  // RUN_TEST( createLockfile_existing );
  // RUN_TEST( createLockfile_both );
  // RUN_TEST( createLockfile_error );

  test_createLockfile_new();

  return exitCode;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
