/* ========================================================================== *
 *
 * @file registry.cc
 *
 * @brief A set of user inputs and preferences used for resolution and search.
 *
 *
 * -------------------------------------------------------------------------- */

#include <nix/flake/flakeref.hh>

#include "flox/core/util.hh"
#include "flox/pkgdb/input.hh"
#include "flox/registry.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

void
InputPreferences::clear()
{
  this->subtrees = std::nullopt;
}


/* -------------------------------------------------------------------------- */

void
InputPreferences::merge( const InputPreferences & overrides )
{
  if ( overrides.subtrees.has_value() )
    {
      if ( this->subtrees.has_value() )
        {
          std::optional<std::vector<Subtree>> merged = std::make_optional(
            flox::merge_vectors( this->subtrees.value(),
                                 overrides.subtrees.value() ) );
          this->subtrees.swap( merged );
        }
      else { this->subtrees = overrides.subtrees; }
    }
}


/* -------------------------------------------------------------------------- */

pkgdb::PkgQueryArgs &
InputPreferences::fillPkgQueryArgs( pkgdb::PkgQueryArgs & pqa ) const
{
  pqa.subtrees = this->subtrees;
  return pqa;
}


/* -------------------------------------------------------------------------- */

void
RegistryRaw::clear()
{
  this->inputs.clear();
  this->priority.clear();
  this->defaults.clear();
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
        {
          try
            {
              value.get_to( rip.subtrees );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidRegistryException(
                "couldn't interpret registry input field `subtrees'",
                flox::extract_json_errmsg( err ) );
            }
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
                "couldn't interpret registry input field `from'",
                flox::extract_json_errmsg( err ) );
            }
        }
      else { throw InvalidRegistryException( "unknown field `" + key + "'" ); }
    }
}


void
to_json( nlohmann::json & jto, const RegistryInput & rip )
{
  jto = {
    { "subtrees", rip.subtrees },
  };
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
                    "couldn't extract input `" + ikey + "'",
                    flox::extract_json_errmsg( err ) );
                }
              inputs.insert( { ikey, input } );
            }
          reg.inputs = inputs;
        }
      else if ( key == "defaults" )
        {
          InputPreferences prefs;
          try
            {
              value.get_to( prefs );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidRegistryException(
                "couldn't extract input preferences",
                flox::extract_json_errmsg( err ) );
            }
          reg.defaults = prefs;
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
          throw InvalidRegistryException( "unrecognized registry field `" + key
                                          + "'" );
        }
    }
}

/** @brief Convert a @a flox::RegistryRaw to a JSON object. */
void
to_json( nlohmann::json & jto, const RegistryRaw & reg )
{
  jto = { { "inputs", reg.inputs },
          { "defaults", reg.defaults },
          { "priority", reg.priority } };
}


/* -------------------------------------------------------------------------- */

pkgdb::PkgQueryArgs &
RegistryRaw::fillPkgQueryArgs( const std::string &   input,
                               pkgdb::PkgQueryArgs & pqa ) const
{
  /* Look for the named input and our fallbacks/default in the inputs list.
   * then fill input specific settings. */
  try
    {
      const RegistryInput & minput = this->inputs.at( input );
      pqa.subtrees = minput.subtrees.has_value() ? minput.subtrees
                                                 : this->defaults.subtrees;
    }
  catch ( ... )
    {
      pqa.subtrees = this->defaults.subtrees;
    }
  return pqa;
}


/* -------------------------------------------------------------------------- */

void
RegistryRaw::merge( const RegistryRaw & overrides )
{
  for ( const auto & [key, value] : overrides.inputs )
    {
      this->inputs[key] = value;
    }
  this->defaults.merge( overrides.defaults );
  this->priority = merge_vectors( this->priority, overrides.priority );
}

/* -------------------------------------------------------------------------- */

nix::ref<FloxFlake>
FloxFlakeInput::getFlake()
{
  if ( this->flake == nullptr )
    {
      auto ref = *this->getFlakeRef();
      this->flake
        = std::make_shared<FloxFlake>( NixState( this->store ).getState(),
                                       ref );
    }
  return static_cast<nix::ref<FloxFlake>>( this->flake );
}


/* -------------------------------------------------------------------------- */

const std::vector<Subtree> &
FloxFlakeInput::getSubtrees()
{
  if ( ! this->enabledSubtrees.has_value() )
    {
      if ( this->subtrees.has_value() )
        {
          this->enabledSubtrees = *this->subtrees;
        }
      else
        {
          try
            {
              auto root = this->getFlake()->openEvalCache()->getRoot();
              if ( root->maybeGetAttr( "packages" ) != nullptr )
                {
                  this->enabledSubtrees = std::vector<Subtree> { ST_PACKAGES };
                }
              else if ( root->maybeGetAttr( "legacyPackages" ) != nullptr )
                {
                  this->enabledSubtrees = std::vector<Subtree> { ST_LEGACY };
                }
              else { this->enabledSubtrees = std::vector<Subtree> {}; }
            }
          catch ( const nix::EvalError & err )
            {
              throw NixEvalException( "could not determine flake subtrees",
                                      err );
            }
        }
    }
  return *this->enabledSubtrees;
}


/* -------------------------------------------------------------------------- */

RegistryInput
FloxFlakeInput::getLockedInput()
{
  return { this->getSubtrees(), this->getFlake()->lockedFlake.flake.lockedRef };
}


/* -------------------------------------------------------------------------- */

std::map<std::string, RegistryInput>
FlakeRegistry::getLockedInputs()
{
  std::map<std::string, RegistryInput> locked;
  for ( auto & [name, input] : *this )
    {
      locked.emplace( name, input->getLockedInput() );
    }
  return locked;
}

void
from_json( const nlohmann::json & jfrom, InputPreferences & prefs )
{
  assertIsJSONObject<InvalidRegistryException>( jfrom, "input preferences" );
  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "subtrees" )
        {
          try
            {
              value.get_to( prefs.subtrees );
            }
          catch ( nlohmann::json::exception & err )
            {
              throw InvalidRegistryException(
                "couldn't interpret field `subtrees'",
                flox::extract_json_errmsg( err ) );
            }
        }
      else { throw InvalidRegistryException( "unknown field `" + key + "'" ); }
    }
}


void
to_json( nlohmann::json & jto, const InputPreferences & prefs )
{
  jto = { { "subtrees", prefs.subtrees } };
}


/* -------------------------------------------------------------------------- */

RegistryRaw
lockRegistry( const RegistryRaw & unlocked, const nix::ref<nix::Store> & store )
{
  auto factory  = FloxFlakeInputFactory( store );
  auto locked   = unlocked;
  locked.inputs = FlakeRegistry( unlocked, factory ).getLockedInputs();
  return locked;
}


/* -------------------------------------------------------------------------- */

RegistryRaw
getGARegistry()
{
  static const std::string refOrRev
    = nix::getEnv( "_PKGDB_GA_REGISTRY_REF_OR_REV" )
        .value_or( "release-23.11" );
  static const nix::FlakeRef nixpkgsRef
    = nix::parseFlakeRef( FLOX_FLAKE_TYPE + ":NixOS/nixpkgs/" + refOrRev );
  if ( nix::lvlTalkative < nix::verbosity )
    {
      nix::logger->log( nix::lvlTalkative,
                        "GA Registry is using `nixpkgs' as `"
                          + nixpkgsRef.to_string() + "'." );
    }
  return RegistryRaw(
    std::map<std::string, RegistryInput> {
      { "nixpkgs",
        RegistryInput( std::vector<Subtree> { ST_LEGACY }, nixpkgsRef ) } },
    std::vector<std::string> { "nixpkgs" } );
}


/* -------------------------------------------------------------------------- */

bool
RegistryRaw::operator==( const RegistryRaw & other ) const
{
  if ( this->priority != other.priority ) { return false; }
  if ( this->defaults != other.defaults ) { return false; }
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
