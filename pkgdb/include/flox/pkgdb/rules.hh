/* ========================================================================== *
 *
 * @file flox/pkgdb/rules.hh
 *
 * @brief Declares `RulesTreeNode' class, `ScrapeRules` helpers, and interfaces
 *        related related to rules processing
 *        for @a flox::pkgdb::PkgDb::scrape().
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <tuple>

#include <nlohmann/json.hpp>

#include "flox/core/types.hh"
#include "flox/pkgdb/read.hh"


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
  // TODO: aliases
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

struct RulesTreeNode
{
  using Children = std::unordered_map<std::string, RulesTreeNode>;

  std::string attrName = "";
  ScrapeRule  rule     = SR_DEFAULT;
  Children    children = {};

  RulesTreeNode() = default;

  explicit RulesTreeNode( ScrapeRulesRaw rules );

  explicit RulesTreeNode( const std::filesystem::path & path )
    : RulesTreeNode( static_cast<ScrapeRulesRaw>( readAndCoerceJSON( path ) ) )
  {}

  explicit RulesTreeNode( std::string attrName,
                          ScrapeRule  rule     = SR_DEFAULT,
                          Children    children = {} )
    : attrName( std::move( attrName ) )
    , rule( std::move( rule ) )
    , children( std::move( children ) )
  {}

  RulesTreeNode( std::string attrName, Children children )
    : attrName( std::move( attrName ) ), children( std::move( children ) )
  {}

  void
  addRule( AttrPathGlob & relPath, ScrapeRule rule );

  /**
   * @brief Get the rule at a path, or @a flox::pkgdb::SR_DEFAULT as a fallback.
   *
   * This *does NOT* apply parent rules to children.
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

  std::string
  getHash() const;


}; /* End struct `RulesTreeNode' */


/** @brief Convert a JSON object to a @a flox::pkgdb::RulesTreeNode. */
void
from_json( const nlohmann::json & jfrom, RulesTreeNode & rules );

/** @brief Convert a @a flox::pkgdb::RulesTreeNode to a JSON object. */
void
to_json( nlohmann::json & jto, const RulesTreeNode & rules );


/* -------------------------------------------------------------------------- */

/**
 * @brief Get the _builtin_ rules set.
 *
 * This default ruleset should be used in all contexts except for testing until
 * we begin supporting _custom catalogs_, _custom builds_, or _custom rules_.
 */
const RulesTreeNode &
getDefaultRules();


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
