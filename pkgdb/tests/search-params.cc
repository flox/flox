/* ========================================================================== *
 *
 * @file tests/search-params.cc
 *
 * @brief Minimal executable that parses a @a flox::search::SearchParams struct.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <cstdlib>
#include <iostream>

#include <nlohmann/json.hpp>

#include "flox/search/params.hh"


/* -------------------------------------------------------------------------- */

using namespace nlohmann::literals;


/* -------------------------------------------------------------------------- */

int
main( int argc, char * argv[] )
{

  /* Parse */
  nlohmann::json paramsJSON;
  if ( argc < 2 )
    {
      try
        {
          std::string line;
          std::string paramsString;

          while ( std::getline( std::cin, line ) && ( ! line.empty() ) )
            {
              paramsString += line;
            }

          paramsJSON = nlohmann::json::parse( paramsString );
        }
      catch ( const std::exception & err )
        {
          std::cerr << "ERROR: Failed to parse search parameters: "
                    << err.what() << std::endl;
          return EXIT_FAILURE + 1;
        }
      catch ( ... )
        {
          std::cerr << "ERROR: Failed to parse search parameters." << std::endl;
          return EXIT_FAILURE + 2;
        }
    }
  else
    {
      try
        {
          paramsJSON = flox::parseOrReadJSONObject( argv[1] );
        }
      catch ( const std::exception & err )
        {
          std::cerr << "ERROR: Failed to parse search parameters: "
                    << err.what() << std::endl;
          return EXIT_FAILURE + 1;
        }
      catch ( ... )
        {
          std::cerr << "ERROR: Failed to parse search parameters." << std::endl;
          return EXIT_FAILURE + 2;
        }
    }


  /* Deserialize */
  flox::search::SearchParams params;
  try
    {
      paramsJSON.get_to( params );
    }
  catch ( const std::exception & err )
    {
      std::cerr << "ERROR: Failed to convert search parameters from JSON: "
                << err.what() << std::endl;
      return EXIT_FAILURE + 3;
    }
  catch ( ... )
    {
      std::cerr << "ERROR: Failed to convert search parameters from JSON."
                << std::endl;
      return EXIT_FAILURE + 4;
    }


  /* Serialize */
  try
    {
      std::cout << nlohmann::json( params ).dump() << std::endl;
    }
  catch ( const std::exception & err )
    {
      std::cerr << "ERROR: Failed to serialize search parameters: "
                << err.what() << std::endl;
      return EXIT_FAILURE + 5;
    }
  catch ( ... )
    {
      std::cerr << "ERROR: Failed to serialize search parameters." << std::endl;
      return EXIT_FAILURE + 6;
    }

  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
