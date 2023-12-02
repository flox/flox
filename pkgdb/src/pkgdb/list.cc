/* ========================================================================== *
 *
 * @file pkgdb/list.cc
 *
 * @brief Implementation of `pkgdb list` subcommand.
 *
 * Used to list a summary of all known `pkgdb` databases.
 *
 *
 * -------------------------------------------------------------------------- */

#include <iostream>

#include "flox/pkgdb/command.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

ListCommand::ListCommand() : parser( "list" )
{
  this->parser.add_description( "Summarize available Package DBs" );

  this->parser.add_argument( "-c", "--cachedir" )
    .help( "summarize databases in a given directory" )
    .metavar( "PATH" )
    .nargs( 1 )
    .action( [&]( const std::string & cacheDir )
             { this->cacheDir = nix::absPath( cacheDir ); } );

  this->parser.add_argument( "-j", "--json" )
    .help( "output as JSON" )
    .nargs( 0 )
    .action( [&]( const std::string & ) { this->json = true; } );

  this->parser.add_argument( "-b", "--basenames" )
    .help( "print basenames of databases instead of absolute paths" )
    .nargs( 0 )
    .action( [&]( const std::string & ) { this->basenames = true; } );
}


/* -------------------------------------------------------------------------- */
int
ListCommand::run()
{
  std::filesystem::path cacheDir
    = this->cacheDir.value_or( getPkgDbCachedir() );

  /* Make sure the cache directory exists. */
  if ( ! std::filesystem::exists( cacheDir ) )
    {
      /* If the user explicitly gave a directory, throw an error. */
      if ( this->cacheDir.has_value() )
        {
          std::cerr << "No such cachedir: " << cacheDir << std::endl;
          return EXIT_FAILURE;
        }
      /* Otherwise "they just don't have any databases", so don't error out." */
      return EXIT_SUCCESS;
    }

  nlohmann::json dbs = nlohmann::json::object();

  /* Show the cachedir path over stderr if we're only printing basenames and
   * they didn't specify it explicitly. */
  if ( this->basenames && ( ! this->cacheDir.has_value() ) )
    {
      std::cerr << "pkgdb cachedir: " << cacheDir.string() << std::endl;
    }

  for ( const auto & entry : std::filesystem::directory_iterator( cacheDir ) )
    {
      if ( ! isSQLiteDb( entry.path() ) ) { continue; }

      PkgDbReadOnly db( entry.path().string() );

      std::string dbPath = this->basenames ? entry.path().filename().string()
                                           : entry.path().string();

      if ( this->json )
        {
          dbs[dbPath] = { { "string", db.lockedRef.string },
                          { "attrs", db.lockedRef.attrs },
                          { "fingerprint",
                            db.fingerprint.to_string( nix::Base16, false ) } };
        }
      else { std::cout << db.lockedRef.string << ' ' << dbPath << std::endl; }
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
