/* ========================================================================== *
 *
 * @file pkgdb/gc.cc
 *
 * @brief Implementation of `pkgdb gc` subcommand.
 *
 * Used to remove stale `pkgdb` databases.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <chrono>
#include <cstdlib>
#include <filesystem>
#include <iostream>
#include <optional>
#include <string>
#include <sys/stat.h>
#include <utime.h>
#include <variant>
#include <vector>

#include <argparse/argparse.hpp>
#include <nix/error.hh>
#include <nix/fmt.hh>
#include <nix/logging.hh>
#include <nix/types.hh>
#include <nix/util.hh>

#include "flox/core/command.hh"
#include "flox/core/exceptions.hh"
#include "flox/core/util.hh"
#include "flox/pkgdb/command.hh"
#include "flox/pkgdb/gc.hh"
#include "flox/pkgdb/read.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

std::vector<std::filesystem::path>
findStaleDatabases( const std::filesystem::path & cacheDir, int minAgeDays )
{

  nix::logger->log( nix::Verbosity::lvlDebug,
                    nix::fmt( "cacheDir: %s\n", cacheDir.c_str() ) );

  std::vector<std::filesystem::path> toDelete;
  for ( const auto & entry : std::filesystem::directory_iterator( cacheDir ) )
    {

      struct stat result
      {};

      if ( stat( entry.path().c_str(), &result ) == 0 )
        {
          auto accessTime
            = std::chrono::system_clock::from_time_t( result.st_atime );

          auto now = std::chrono::system_clock::now();

          auto ageInDays
            = std::chrono::duration_cast<std::chrono::days>( now - accessTime )
                .count();

          nix::logger->log(
            nix::Verbosity::lvlDebug,
            nix::fmt( "%s: atime: %ld, now: %ld, age: %d\n",
                      entry.path().c_str(),
                      result.st_atime,
                      std::chrono::system_clock::to_time_t( now ),
                      ageInDays ) );


          if ( ( minAgeDays <= ageInDays ) && isSQLiteDb( entry.path() ) )
            {
              toDelete.push_back( entry.path() );
            }

          struct utimbuf newTimes
          {};
          newTimes.actime = std::chrono::system_clock::to_time_t( accessTime );
          utime( entry.path().c_str(), &newTimes );
        }
    }
  return toDelete;
}

/* -------------------------------------------------------------------------- */

GCCommand::GCCommand() : parser( "gc" )
{
  this->parser.add_description( "Delete stale Package DBs" );

  this->parser.add_argument( "-c", "--cachedir" )
    .help( "delete databases in a given directory" )
    .metavar( "PATH" )
    .nargs( 1 )
    .default_value( getPkgDbCachedir() )
    .action( [&]( const std::string & cacheDir )
             { this->cacheDir = nix::absPath( cacheDir ); } );

  this->parser.add_argument( "-a", "--min-age" )
    .help( "minimum age in days" )
    .metavar( "AGE" )
    .nargs( 1 )
    .default_value( GCCommand::DEF_STALE_AGE_IN_DAYS )
    .action( [&]( const std::string & minAgeStr )
             { this->gcStaleAgeDays = stoi( minAgeStr ); } );

  this->parser.add_argument( "--dry-run" )
    .help( "list which databases are deleted, but don't actually delete them" )
    .default_value( false )
    .implicit_value( true )
    .action( [&]( const auto & ) { this->dryRun = true; } );
}


/* -------------------------------------------------------------------------- */

int
GCCommand::run()
{
  std::filesystem::path cacheDir
    = this->cacheDir.value_or( getPkgDbCachedir() );

  /* Make sure the cache directory exists. */
  if ( ! std::filesystem::exists( cacheDir ) )
    {
      /* If the user explicitly gave a directory, throw an error. */
      if ( this->cacheDir.has_value() )
        {
          throw FloxException( "no such cachedir: '" + cacheDir.string()
                               + "'" );
          return EXIT_FAILURE;
        }
      /* Otherwise "they just don't have any databases", so don't error out." */
      return EXIT_SUCCESS;
    }

  auto toDelete = findStaleDatabases( cacheDir, this->gcStaleAgeDays );

  std::cout << "Found " << toDelete.size() << " stale databases." << '\n';
  for ( const auto & path : toDelete )
    {
      std::cout << "deleting " << path;
      if ( this->dryRun ) { std::cout << " (dry run)" << '\n'; }
      else
        {
          std::cout << '\n';
          std::filesystem::remove( path );
        }
    }

  return EXIT_SUCCESS;
}

/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
