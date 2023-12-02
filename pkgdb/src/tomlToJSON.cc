/* ========================================================================== *
 *
 * @file tomlToJSON.cc
 *
 * @brief Convert a TOML string to a JSON object.
 *
 *
 * -------------------------------------------------------------------------- */

#include <string>

#include <nlohmann/json.hpp>
#include <toml.hpp>

#include "flox/core/exceptions.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/**
 * @class flox::TOMLToJSONException
 * @brief An exception thrown when converting TOML to JSON.
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( TOMLToJSONException,
                       EC_TOML_TO_JSON,
                       "error converting TOML to JSON" )
/** @} */


/* -------------------------------------------------------------------------- */

nlohmann::json
tomlToJSON( std::string_view toml )
{
  std::string        tomlStr( toml );
  std::istringstream tomlStream( tomlStr );

  std::function<void( nlohmann::json &, const toml::value & )> visit;

  visit = [&]( nlohmann::json & jto, const toml::value & tfrom )
  {
    switch ( tfrom.type() )
      {
          case toml::value_t::table: {
            jto = nlohmann::json::object();
            for ( const auto & elem : toml::get<toml::table>( tfrom ) )
              {
                nlohmann::json jval;
                visit( jval, elem.second );
                jto.emplace( elem.first, std::move( jval ) );
              }
          }
          break;

          case toml::value_t::array: {
            auto tlist = toml::get<std::vector<toml::value>>( tfrom );
            jto        = nlohmann::json::array();
            for ( const auto & elem : tlist )
              {
                nlohmann::json jval;
                visit( jval, elem );
                jto.emplace_back( std::move( jval ) );
              }
          }
          break;

        case toml::value_t::boolean: jto = toml::get<bool>( tfrom ); break;

        case toml::value_t::integer: jto = toml::get<int64_t>( tfrom ); break;

        case toml::value_t::floating: jto = toml::get<double>( tfrom ); break;

        case toml::value_t::string:
          jto = toml::get<std::string>( tfrom );
          break;

        case toml::value_t::empty: jto = nullptr; break;

        case toml::value_t::local_datetime:
        case toml::value_t::offset_datetime:
        case toml::value_t::local_date:
        case toml::value_t::local_time:
          throw std::runtime_error( "Dates and times are not supported" );
          break;

        default: throw std::runtime_error( "Unknown TOML type" ); break;
      }
  }; /* End lambda `visit() */

  try
    {
      auto toml = toml::parse( tomlStream, "tomlToJSON" /* the "filename" */ );
      nlohmann::json rsl;
      visit( rsl, toml );
      return rsl;
    }
  catch ( const std::exception & e )  // TODO: toml::syntax_error
    {
      throw TOMLToJSONException( "while parsing a TOML string", e.what() );
    }
  catch ( ... )
    {
      throw TOMLToJSONException( "while parsing a TOML string" );
    }

  assert( false ); /* Unreachable */
  return nlohmann::json();

} /* End fn `tomlToJSON()' */


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
