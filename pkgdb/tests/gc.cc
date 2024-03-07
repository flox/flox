/* ========================================================================== *
 *
 * @file gc.cc
 *
 * @brief Tests for `flox` garbage collection.
 *
 *
 * -------------------------------------------------------------------------- */

#include <fstream>
#include <sys/stat.h>
#include <utime.h>

#include "flox/pkgdb/db-package.hh"
#include "flox/pkgdb/gc.hh"
#include "flox/pkgdb/write.hh"

#include "test.hh"


/* -------------------------------------------------------------------------- */

bool
test_findStaleDb()
{
  /* Initialize `nix'. */
  flox::NixState nstate;

  auto tempdir     = nix::createTempDir();
  auto currentPath = std::string( tempdir ).append( "/current.db" );
  auto stalePath   = std::string( tempdir ).append( "/stale.db" );


  nix::FlakeRef   ref = nix::parseFlakeRef( nixpkgsRef );
  flox::FloxFlake flake( nstate.getState(), ref );

  flox::pkgdb::PkgDb current
    = flox::pkgdb::PkgDb( flake.lockedFlake, currentPath );

  flox::pkgdb::PkgDb stale = flox::pkgdb::PkgDb( flake.lockedFlake, stalePath );

  /* Set access time of stale.db to 3 days ago. */
  {
    struct stat    statValue;
    struct utimbuf newTimes;
    stat( stalePath.c_str(), &statValue );
    auto atime = std::chrono::system_clock::from_time_t( statValue.st_atime )
                 - std::chrono::days( 4 );

    newTimes.actime  = std::chrono::system_clock::to_time_t( atime );
    newTimes.modtime = statValue.st_mtime;

    // /* For debugging */
    // printf( "setting access time of %s to %ld\n",
    //         stalePath.c_str(),
    //         newTimes.actime );

    utime( stalePath.c_str(), &newTimes );

    stat( stalePath.c_str(), &statValue );
    // /* For debugging */
    // printf( "access time of %s is now %ld\n",
    //         stalePath.c_str(),
    //         statValue.st_atime );
  }

  auto toDelete = flox::pkgdb::findStaleDatabases( tempdir, 3 );

  EXPECT_EQ( toDelete.size(), 1u );

  EXPECT_EQ( toDelete.at( 0 ).compare( stalePath ), 0 );

  return true;
}


/* -------------------------------------------------------------------------- */

int
main( int argc, char * argv[] )
{
  int exitStatus = EXIT_SUCCESS;
#define RUN_TEST( ... ) _RUN_TEST( exitStatus, __VA_ARGS__ )

  nix::verbosity = nix::lvlWarn;
  if ( ( 1 < argc ) && ( std::string_view( argv[1] ) == "-v" ) )
    {
      nix::verbosity = nix::lvlDebug;
    }

  RUN_TEST( findStaleDb );

  return exitStatus;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
