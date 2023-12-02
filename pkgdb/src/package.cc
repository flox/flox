/* ========================================================================== *
 *
 * @file package.cc
 *
 * @brief Abstract representation of a package.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nlohmann/json.hpp>

#include "flox/package.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

std::string
Package::toURIString( const nix::FlakeRef & ref ) const
{
  std::stringstream uri;
  uri << ref.to_string() << "#";
  AttrPath pathS = this->getPathStrs();
  for ( size_t i = 0; i < pathS.size(); ++i )
    {
      uri << '"' << pathS.at( i );
      if ( ( i + 1 ) < pathS.size() ) { uri << "."; }
    }
  return uri.str();
}


/* -------------------------------------------------------------------------- */

nlohmann::json
Package::getInfo( bool withDescription ) const
{
  System system = this->getPathStrs().at( 1 );

  nlohmann::json jto = { { system,
                           { { "name", this->getFullName() },
                             { "pname", this->getPname() } } } };

  std::optional<std::string> oos = this->getVersion();

  if ( oos.has_value() ) { jto[system].emplace( "version", *oos ); }
  else { jto[system].emplace( "version", nlohmann::json() ); }

  oos = this->getSemver();
  if ( oos.has_value() ) { jto[system].emplace( "semver", *oos ); }
  else { jto[system].emplace( "semver", nlohmann::json() ); }

  jto[system].emplace( "outputs", this->getOutputs() );
  jto[system].emplace( "outputsToInstall", this->getOutputsToInstall() );

  oos = this->getLicense();
  if ( oos.has_value() ) { jto[system].emplace( "license", *oos ); }
  else { jto[system].emplace( "license", nlohmann::json() ); }

  std::optional<bool> obool = this->isBroken();
  if ( obool.has_value() ) { jto[system].emplace( "broken", *obool ); }
  else { jto[system].emplace( "broken", nlohmann::json() ); }

  obool = this->isUnfree();
  if ( obool.has_value() ) { jto[system].emplace( "unfree", *obool ); }
  else { jto[system].emplace( "unfree", nlohmann::json() ); }

  if ( withDescription )
    {
      std::optional<std::string> odesc = this->getDescription();
      if ( odesc.has_value() ) { jto[system].emplace( "description", *odesc ); }
      else { jto[system].emplace( "description", nlohmann::json() ); }
    }

  return jto;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
