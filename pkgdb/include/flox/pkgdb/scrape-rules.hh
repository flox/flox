/* ========================================================================== *
 *
 * @file flox/pkgdb/scrape-rules.hh
 *
 * @brief Interfaces for using rules during scraping.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <stack>

#include <nlohmann/json.hpp>

#include <nix/hash.hh>

#include "flox/core/types.hh"
#include "flox/core/util.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/** @brief Scraping rules to modify database creation process in _raw_ form. */
struct ScrapeRulesRaw
{
  std::vector<AttrPathGlob> allowPackage;
  std::vector<AttrPathGlob> disallowPackage;
  std::vector<AttrPathGlob> allowRecursive;
  std::vector<AttrPathGlob> disallowRecursive;
}; /* End struct `ScrapeRulesRaw` */


/** @brief Convert a JSON object to a @a flox::pkgdb::ScrapeRulesRaw. */
void
from_json( const nlohmann::json & jfrom, ScrapeRulesRaw & rules );


/* -------------------------------------------------------------------------- */

enum ScrapeRule {
  SR_NONE = 0,         /**< Empty state. */
  SR_DEFAULT,          /**< Applies no special rules. */
  SR_ALLOW_PACKAGE,    /**< Forces an package entry in DB. */
  SR_ALLOW_RECURSIVE,  /**< Forces a sub-tree to be scraped. */
  SR_DISALLOW_PACKAGE, /**< Do not add package entry to DB. */
  /** Ignore sub-tree members unless otherwise specified. */
  SR_DISALLOW_RECURSIVE
}; /* End enum `ScrapeRule` */

[[nodiscard]] std::string
scrapeRuleToString( ScrapeRule rule );


/* -------------------------------------------------------------------------- */

/**
 * @brief Node definition for a rules tree.
 *
 * The tree is built with a root node, where each node contains an attribute
 * name, and the rule to be applied, along with a list of child nodes.
 * This tree is built from reading the rules file, with paths through the tree
 * constructed with SR_DEFAULT rules along the path until a leaf node with the
 * appropriate rule can be added.  This allows hierarchical searching through
 * the tree for attribute paths encountered during scraping and maintains the
 * context for child inheritance of the rule defined for the deepest ancestral
 * node.  The rules tree is built as such entirely, once by reading the rules
 * file. Attributes are checked node by node, until the full attribute lands on
 * a node with a rule, or SR_DEFAULT is returned, instructing scrape to use the
 * default decision making process.
 *
 * Example, the following 2 rules result in the following tree:
 *
 * allowRecursive foo.bar.bat
 * allowRecursive foo.boo
 *
 * _root -> SR_DEFAULT
 *   ^- foo -> SR_DEFAULT
 *     ^- boo -> SR_ALLOW_RECURSIVE
 *     ^- bar -> SR_DEFAULT
 *       ^- bat -> SR_ALLOW_RECURSIVE
 */
struct RulesTreeNode
{
  using Children = std::unordered_map<std::string, RulesTreeNode>;

  std::string attrName;
  ScrapeRule  rule     = SR_DEFAULT;
  Children    children = {};

  RulesTreeNode() = default;

  explicit RulesTreeNode( const ScrapeRulesRaw & raw );

  explicit RulesTreeNode( const std::filesystem::path & path )
    : RulesTreeNode(
      static_cast<const ScrapeRulesRaw &>( readAndCoerceJSON( path ) ) )
  {}

  explicit RulesTreeNode( std::string attrName,
                          ScrapeRule  rule     = SR_DEFAULT,
                          Children    children = {} )
    : attrName( std::move( attrName ) )
    , rule( rule )
    , children( std::move( children ) )
  {}

  RulesTreeNode( std::string attrName, Children children )
    : attrName( std::move( attrName ) ), children( std::move( children ) )
  {}

  /**
   * @brief Adds a single rule to the RulesTree.
   *
   * This will add a node at @a relPath, relative to this node with the given
   * rule, setting new descendant nodes to SR_DEFAULT along the way.  Trying to
   * overwrite an existing rule that is not SR_DEFAULT will throw an exception.
   *
   * @see @a flox::pkgdb::RulesTreeNode::applyRules
   */
  void
  addRule( AttrPathGlob & relPath, ScrapeRule rule );

  /**
   * @brief Get the rule at a path, or @a flox::pkgdb::SR_DEFAULT as a fallback.
   *
   * This *does NOT* apply parent rules to children.  The @a path is considered
   * to be relative to this node.
   *
   * @see @a flox::pkgdb::RulesTreeNode::applyRules
   */
  [[nodiscard]] ScrapeRule
  getRule( const AttrPath & path = {} ) const;

  /**
   * @brief Return true/false for explicit allow/disallow, or `std::nullopt`
   *        if no rule is defined.
   *        This is intended for use on _root_ nodes.
   *
   * Parent paths may _pass down_ rules to children unless otherwise defined
   * at lower levels.
   */
  [[nodiscard]] std::optional<bool>
  applyRules( const AttrPath & path ) const;


}; /* End struct `RulesTreeNode' */


/* -------------------------------------------------------------------------- */

/**
 * @brief Class to encapsulte a set of scraping rules
 *
 * This includes a root @a RulesTreeNode and a hash of the rules string that
 * created it.
 */
class ScrapeRules
{
public:

  /**
   * @brief Creates a Rules tree and associated hash from a given string
   * representaion of the rules JSON data.
   *
   */
  explicit ScrapeRules( const std::string_view & rulesJSON );

  /**
   * @brief Applies the rules of the tree to the @a path provided.  See @a
   * RulesTreeNode::applyRules() for further details.
   *
   */
  [[nodiscard]] std::optional<bool>
  applyRules( const AttrPath & path ) const
  {
    return rootNode.applyRules( path );
  }

  /**
   * @brief Returns the root tree node of the rules tree.
   *
   */
  const RulesTreeNode &
  getRootNode()
  {
    return rootNode;
  }

  /**
   * @brief Returns a hash in string format of the rules tree.
   *
   */
  std::string
  hashString() const
  {
    return hash.to_string( nix::Base16, true );
  }

private:

  RulesTreeNode rootNode;
  nix::Hash     hash;
}; /* End clss `ScrapeRules' */

/** @brief Convert a JSON object to a @a flox::pkgdb::RulesTreeNode. */
void
from_json( const nlohmann::json & jfrom, RulesTreeNode & rules );

/** @brief Convert a @a flox::pkgdb::RulesTreeNode to a JSON object. */
void
to_json( nlohmann::json & jto, const RulesTreeNode & rules );

const ScrapeRules &
getDefaultRules();

/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
