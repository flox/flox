/* ========================================================================== *
 *
 * @file yamlToJSON.cc
 *
 * @brief Convert a YAML string to a JSON object.
 *
 *
 * -------------------------------------------------------------------------- */

#include <string>

#include <nlohmann/json.hpp>
#include <yaml-cpp/yaml.h>

#include "flox/core/exceptions.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/**
 * @class flox::YAMLToJSONException
 * @brief An exception thrown when converting YAML to JSON.
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( YAMLToJSONException,
                       EC_YAML_TO_JSON,
                       "error converting YAML to JSON" )
/** @} */


/* -------------------------------------------------------------------------- */

nlohmann::json
yamlToJSON( std::string_view yaml )
{
  std::function<void( nlohmann::json &, const YAML::Node & )> visit;

  visit = [&]( nlohmann::json & jto, const YAML::Node & yfrom )
  {
    switch ( yfrom.Type() )
      {
        case YAML::NodeType::Null: jto = nullptr; break;

        case YAML::NodeType::Scalar:
          /* Detect integers, floats, bools, and real strings! */
          try
            {
              jto = yfrom.as<int64_t>();
            }
          catch ( ... )
            {
              try
                {
                  jto = yfrom.as<double>();
                }
              catch ( ... )
                {
                  try
                    {
                      jto = yfrom.as<bool>();
                    }
                  catch ( ... )
                    {
                      jto = yfrom.as<std::string>();
                    }
                }
            }
          break;

        case YAML::NodeType::Sequence:
          jto = nlohmann::json::array();
          for ( const auto & elem : yfrom )
            {
              nlohmann::json jval;
              visit( jval, elem );
              jto.emplace_back( std::move( jval ) );
            }
          break;

        case YAML::NodeType::Map:
          jto = nlohmann::json::object();
          for ( const auto & elem : yfrom )
            {
              nlohmann::json jval;
              visit( jval, elem.second );
              jto.emplace( elem.first.as<std::string>(), std::move( jval ) );
            }
          break;

        case YAML::NodeType::Undefined:
          throw YAMLToJSONException( "YAML node has an undefined type" );
          break;

        default:
          throw YAMLToJSONException( "YAML node has an unrecognized type" );
          break;
      }
  }; /* End fn `visit()' */

  try
    {
      std::string    yamlStr( yaml );
      YAML::Node     yaml = YAML::Load( yamlStr );
      nlohmann::json rsl;
      visit( rsl, yaml );
      return rsl;
    }
  catch ( const std::exception & e )
    {
      throw YAMLToJSONException( "while parsing a YAML string", e.what() );
    }
  catch ( ... )
    {
      throw YAMLToJSONException( "while parsing a YAML string" );
    }

  assert( false ); /* Unreachable */
  return nlohmann::json();

} /* End fn `yamlToJSON()' */


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
