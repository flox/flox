/* ========================================================================== *
 *
 * @file registry.cc
 *
 * @brief A set of user inputs and preferences used for resolution and search.
 *
 *
 * -------------------------------------------------------------------------- */

#include <sys/wait.h>

#include <nix/flake/flakeref.hh>

#include "flox/core/util.hh"
#include "flox/registry.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

void
RegistryRaw::clear()
{
  this->inputs.clear();
  this->priority.clear();
}


/* -------------------------------------------------------------------------- */

std::vector<std::reference_wrapper<const std::string>>
RegistryRaw::getOrder() const
{
  std::vector<std::reference_wrapper<const std::string>> order(
    this->priority.cbegin(),
    this->priority.cend() );
  for ( const auto & [key, _] : this->inputs )
    {
      if ( std::find( this->priority.begin(), this->priority.end(), key )
           == this->priority.end() )
        {
          order.emplace_back( key );
        }
    }
  return order;
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, RegistryInput & rip )
{
  assertIsJSONObject<InvalidRegistryException>( jfrom, "registry input" );
  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "subtrees" )
        { /* obsolete field */
        }
      else if ( key == "from" )
        {
          try
            {
              nix::FlakeRef ref = value.get<nix::FlakeRef>();
              rip.from          = std::make_shared<nix::FlakeRef>( ref );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidRegistryException(
                "couldn't interpret registry input field 'from'",
                flox::extract_json_errmsg( err ) );
            }
        }
      else { throw InvalidRegistryException( "unknown field '" + key + "'" ); }
    }
}


void
to_json( nlohmann::json & jto, const RegistryInput & rip )
{
  if ( rip.from == nullptr ) { jto.emplace( "from", nullptr ); }
  else
    {
      jto.emplace( "from", nix::fetchers::attrsToJSON( rip.from->toAttrs() ) );
    }
}

/* -------------------------------------------------------------------------- */

/** @brief Convert a JSON object to a @a flox::RegistryRaw. */
void
from_json( const nlohmann::json & jfrom, RegistryRaw & reg )
{
  assertIsJSONObject<InvalidRegistryException>( jfrom, "registry" );
  reg.clear();
  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( value.is_null() ) { continue; }
      if ( key == "inputs" )
        {
          std::map<std::string, RegistryInput> inputs;
          for ( const auto & [ikey, ivalue] : value.items() )
            {
              RegistryInput input;
              try
                {
                  ivalue.get_to( input );
                }
              catch ( nlohmann::json::exception & err )
                {
                  throw InvalidRegistryException(
                    "couldn't extract input '" + ikey + "'",
                    flox::extract_json_errmsg( err ) );
                }
              inputs.insert( { ikey, input } );
            }
          reg.inputs = inputs;
        }
      else if ( key == "defaults" )
        { /* obsolete field */
        }
      else if ( key == "priority" )
        {
          std::vector<std::string> priority;
          try
            {
              value.get_to( priority );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidRegistryException(
                "couldn't extract input priority",
                flox::extract_json_errmsg( err ) );
            }
          reg.priority = std::move( priority );
        }
      else
        {
          throw InvalidRegistryException( "unrecognized registry field '" + key
                                          + "'" );
        }
    }
}

/** @brief Convert a @a flox::RegistryRaw to a JSON object. */
void
to_json( nlohmann::json & jto, const RegistryRaw & reg )
{
  jto = { { "inputs", reg.inputs }, { "priority", reg.priority } };
}


/* -------------------------------------------------------------------------- */

void
RegistryRaw::merge( const RegistryRaw & overrides )
{
  for ( const auto & [key, value] : overrides.inputs )
    {
      this->inputs[key] = value;
    }
  this->priority = merge_vectors( this->priority, overrides.priority );
}


/* -------------------------------------------------------------------------- */

bool
RegistryRaw::operator==( const RegistryRaw & other ) const
{
  if ( this->priority != other.priority ) { return false; }
  // NOLINTNEXTLINE(readability-use-anyofallof)
  for ( const auto & [key, value] : this->inputs )
    {
      try
        {
          if ( other.inputs.at( key ) != value ) { return false; }
        }
      catch ( ... )
        {
          return false;
        }
    }
  return true;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
