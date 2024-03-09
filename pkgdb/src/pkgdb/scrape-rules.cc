
/* ========================================================================== *
 *
 * @file pkgdb/scrape-rules.cc
 *
 * @brief Implementation for rules used for scraping.
 *
 *
 * -------------------------------------------------------------------------- */

#include <optional>
#include <string>

#include <nlohmann/json.hpp>

#include "flox/pkgdb/scrape-rules.hh"
#include "flox/pkgdb/write.hh"

/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

std::string
scrapeRuleToString( ScrapeRule rule )
{
  switch ( rule )
    {
      case SR_NONE: return "UNSET";
      case SR_DEFAULT: return "default";
      case SR_ALLOW_PACKAGE: return "allowPackage";
      case SR_DISALLOW_PACKAGE: return "disallowPackage";
      case SR_ALLOW_RECURSIVE: return "allowRecursive";
      case SR_DISALLOW_RECURSIVE: return "disallowRecursive";
      default:
        throw PkgDbException( "encountered unexpected rule '"
                              + scrapeRuleToString( rule ) + '\'' );
    }
}


/* -------------------------------------------------------------------------- */

void
RulesTreeNode::addRule( AttrPathGlob & relPath, ScrapeRule rule )
{
  /* Modify our rule. */
  if ( relPath.empty() )
    {
      if ( this->rule != SR_DEFAULT )
        {
          // TODO: Pass abs-path
          throw FloxException( "attempted to overwrite existing rule '"
                               + scrapeRuleToString( this->rule ) + "' for '"
                               + this->attrName + "' with new rule '"
                               + scrapeRuleToString( rule ) + "'" );
        }
      traceLog( "assigning rule to '" + scrapeRuleToString( rule ) + "' to `"
                + this->attrName + '\'' );
      this->rule = rule;
      return;
    }
  traceLog( "adding rule to '" + this->attrName + "': '"
            + displayableGlobbedPath( relPath )
            + "' = " + scrapeRuleToString( rule ) + '\'' );

  /* Handle system glob by splitting into 4 recursive calls. */
  if ( ! relPath.front().has_value() )
    {
      if ( this->attrName != "legacyPackages" )
        {
          throw FloxException(
            "glob in rules (null) only allowed as child of legacyPackages" );
        }

      traceLog( "splitting system glob into real systems" );
      for ( const auto & system : getDefaultSystems() )
        {
          AttrPathGlob relPathCopy = relPath;
          relPathCopy.front()      = system;
          this->addRule( relPathCopy, rule );
        }
      return;
    }

  std::string attrName = std::move( *relPath.front() );
  // TODO: Use a `std::deque' instead of `std::vector' for efficiency.
  //       This container is only used for `push_back' and `pop_front'.
  //       Removing from the front is inefficient for `std::vector'.
  relPath.erase( relPath.begin() );

  if ( auto itChild = this->children.find( attrName );
       itChild != this->children.end() )
    {
      traceLog( "found existing child '" + attrName + '\'' );
      /* Add to existing child node. */
      itChild->second.addRule( relPath, rule );
    }
  else if ( relPath.empty() )
    {
      /* Add leaf node. */
      traceLog( "creating leaf '" + attrName
                + "' = " + scrapeRuleToString( rule ) + '\'' );
      this->children.emplace( attrName, RulesTreeNode( attrName, rule ) );
    }
  else
    {
      traceLog( "creating child '" + attrName + '\'' );
      /* Create a new child node. */
      this->children.emplace( attrName, RulesTreeNode( attrName ) );
      this->children.at( attrName ).addRule( relPath, rule );
    }
}


/* -------------------------------------------------------------------------- */

ScrapeRule
RulesTreeNode::getRule( const AttrPath & path ) const
{
  const RulesTreeNode * node = this;
  for ( const auto & attrName : path )
    {
      try
        {
          node = &node->children.at( attrName );
        }
      catch ( const std::out_of_range & err )
        {
          return SR_DEFAULT;
        }
    }
  return node->rule;
}


/* -------------------------------------------------------------------------- */

std::optional<bool>
RulesTreeNode::applyRules( const AttrPath & path ) const
{
  auto rule = this->getRule( path );
  /* Perform lookup in parents if necessary. */
  if ( rule == SR_DEFAULT )
    {
      AttrPath pathCopy = path;
      while ( ( rule == SR_DEFAULT ) && ( ! pathCopy.empty() ) )
        {
          pathCopy.pop_back();
          rule = this->getRule( pathCopy );
        }
    }

  switch ( rule )
    {
      case SR_ALLOW_PACKAGE: return true;
      case SR_DISALLOW_PACKAGE: return false;
      case SR_ALLOW_RECURSIVE: return true;
      case SR_DISALLOW_RECURSIVE: return false;
      case SR_DEFAULT: return std::nullopt;
      default:
        throw PkgDbException( "encountered unexpected rule `"
                              + scrapeRuleToString( rule ) + '\'' );
    }
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, RulesTreeNode & rules )
{
  ScrapeRulesRaw raw = jfrom;
  rules              = static_cast<RulesTreeNode>( raw );
}


/* -------------------------------------------------------------------------- */

void
to_json( nlohmann::json & jto, const RulesTreeNode & rules )
{
  jto = { { "__rule", scrapeRuleToString( rules.rule ) } };
  for ( const auto & [name, child] : rules.children )
    {
      nlohmann::json jchild;
      to_json( jchild, child );
      jto[name] = jchild;
    }
}


/* -------------------------------------------------------------------------- */

RulesTreeNode::RulesTreeNode( const ScrapeRulesRaw & raw )
{
  /* Add rules in order of precedence */
  for ( const auto & path : raw.allowPackage )
    {
      AttrPathGlob pathCopy( path );
      this->addRule( pathCopy, SR_ALLOW_PACKAGE );
    }
  for ( const auto & path : raw.disallowPackage )
    {
      AttrPathGlob pathCopy( path );
      this->addRule( pathCopy, SR_DISALLOW_PACKAGE );
    }
  for ( const auto & path : raw.allowRecursive )
    {
      AttrPathGlob pathCopy( path );
      this->addRule( pathCopy, SR_ALLOW_RECURSIVE );
    }
  for ( const auto & path : raw.disallowRecursive )
    {
      AttrPathGlob pathCopy( path );
      this->addRule( pathCopy, SR_DISALLOW_RECURSIVE );
    }
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, ScrapeRulesRaw & rules )
{
  auto addPaths
    = []( std::string key, std::vector<AttrPathGlob> vect, auto paths )
  {
    for ( const auto & path : paths )
      {
        try
          {
            vect.emplace_back( path );
          }
        catch ( nlohmann::json::exception & err )
          {
            throw PkgDbException( "couldn't interpret field '" + key + "': ",
                                  flox::extract_json_errmsg( err ) );
          }
      }
  };

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "allowPackage" )
        {
          addPaths( key, rules.allowPackage, value );
        }
      else if ( key == "disallowPackage" )
        {
          addPaths( key, rules.disallowPackage, value );
        }
      else if ( key == "allowRecursive" )
        {
          addPaths( key, rules.allowRecursive, value );
        }
      else if ( key == "disallowRecursive" )
        {
          addPaths( key, rules.disallowRecursive, value );
        }
      else { throw FloxException( "unknown scrape rule: '" + key + "'" ); }
    }
}

/* -------------------------------------------------------------------------- */

ScrapeRules::ScrapeRules( const std::string_view & rulesJSON )
  : hash( nix::hashString( nix::htMD5, rulesJSON ) )
{
  ScrapeRulesRaw raw = nlohmann::json::parse( rulesJSON );
  this->rootNode     = RulesTreeNode( std::move( raw ) );
}

/* Currently returns the one and only set of rules for scraping.
 * These are hardcoded for now.
 * TODO: make the rules file to use a command line argument or otherwise
 * configurable.
 */
const ScrapeRules &
getDefaultRules()
{
  static std::optional<ScrapeRules> rules;

  /* These are just hardcoded for now.*/
  std::string_view rulesJSON = (
#include "./rules.json.hh"
  );

  if ( ! rules.has_value() ) { rules = ScrapeRules( rulesJSON ); }
  return *rules;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
