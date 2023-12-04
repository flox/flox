#include "flox/pkgdb/gc.hh"
#include "flox/pkgdb/db-package.hh"
#include "flox/pkgdb/write.hh"

#include "test.hh"
#include <fstream>
#include <sys/stat.h>
#include <utime.h>

bool
test_findStaleDb()
{
  /* Initialize `nix' */
  flox::NixState nstate;

  auto tempdir      = nix::createTempDir();
  auto current_path = std::string( tempdir ).append( "/current.db" );
  auto stale_path   = std::string( tempdir ).append( "/stale.db" );


  nix::FlakeRef   ref = nix::parseFlakeRef( nixpkgsRef );
  flox::FloxFlake flake( nstate.getState(), ref );

  flox::pkgdb::PkgDb current
    = flox::pkgdb::PkgDb( flake.lockedFlake, current_path );

  flox::pkgdb::PkgDb stale
    = flox::pkgdb::PkgDb( flake.lockedFlake, stale_path );


  {  // set access time of stale.db to 3 days ago
    struct stat    statValue;
    struct utimbuf new_times;
    stat( stale_path.c_str(), &statValue );
    auto atime = std::chrono::system_clock::from_time_t( statValue.st_atime )
                 - std::chrono::days( 4 );

    new_times.actime  = std::chrono::system_clock::to_time_t( atime );
    new_times.modtime = statValue.st_mtime;

    printf( "setting access time of %s to %ld\n",
            stale_path.c_str(),
            new_times.actime );

    utime( stale_path.c_str(), &new_times );

    stat( stale_path.c_str(), &statValue );
    printf( "access time of %s is now %ld\n",
            stale_path.c_str(),
            statValue.st_atime );
  }


  auto to_delete = flox::pkgdb::findStaleDatabases( tempdir, 3 );

  EXPECT_EQ( to_delete.size(), 1u );

  EXPECT_EQ( to_delete.at( 0 ).compare( stale_path ), 0 );

  return true;
}

int
main( int argc, char * argv[] )
{
  int ec = EXIT_SUCCESS;
#define RUN_TEST( ... ) _RUN_TEST( ec, __VA_ARGS__ )

  nix::verbosity = nix::lvlWarn;
  if ( ( 1 < argc ) && ( std::string_view( argv[1] ) == "-v" ) )
    {
      nix::verbosity = nix::lvlDebug;
    }

  /* Initialize `nix' */
  flox::NixState nstate;

  {
    RUN_TEST( findStaleDb );
  }
}
