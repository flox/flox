/* ========================================================================== *
 *
 * @file tests/is_sqlite3.cc
 *
 * @brief Minimal executable to detect if a path is a SQLite3 database.
 *
 *
 * -------------------------------------------------------------------------- */

#include <cstdlib>
#include <iostream>

#include "flox/core/util.hh"


/* -------------------------------------------------------------------------- */

int
main( int argc, char * argv[] )
{
  if ( argc < 2 )
    {
      std::cerr << "You must provide a path argument." << std::endl;
      return EXIT_FAILURE;
    }
  return flox::isSQLiteDb( std::string( argv[1] ) ) ? EXIT_SUCCESS
                                                    : EXIT_FAILURE;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
