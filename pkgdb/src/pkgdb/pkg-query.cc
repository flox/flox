/* ========================================================================== *
 *
 * @file pkgdb/pkg-query.cc
 *
 * @brief Interfaces for constructing complex 'Packages' queries.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <cstddef>
#include <list>
#include <memory>
#include <optional>
#include <regex>
#include <sstream>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>

#include <nix/config.hh>
#include <nix/globals.hh>
#include <nlohmann/json.hpp>
#include <sqlite3pp.hh>

#include "flox/core/types.hh"
#include "flox/core/util.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "versions.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

void
PkgQueryArgs::check() const
{

  if ( this->name.has_value()
       && ( this->pname.has_value() || this->version.has_value()
            || this->semver.has_value() ) )
    {
      throw InvalidPkgQueryArg(
        "queries may not mix 'name' parameter with any of 'pname', "
        "'version', or 'semver' parameters." );
    }

  if ( this->version.has_value() && this->semver.has_value() )
    {
      throw InvalidPkgQueryArg(
        "queries may not mix 'version' and 'semver' parameters." );
    }

  /* Check licenses don't contain the ' character */
  if ( this->licenses.has_value() )
    {
      for ( const auto & license : *this->licenses )
        {
          if ( license.find( '\'' ) != std::string::npos )
            {
              throw InvalidPkgQueryArg(
                "license contains illegal character \"'\": " + license );
            }
        }
    }

  /* Systems */
  for ( const auto & system : this->systems )
    {
      if ( std::find( flox::getDefaultSystems().begin(),
                      flox::getDefaultSystems().end(),
                      system )
           == flox::getDefaultSystems().end() )
        {

          throw InvalidPkgQueryArg( "unrecognized or unsupported system: "
                                    + std::string( system ) );
        }
    }

  /* `partialMatch' and `partialNameMatch' cannot be used together. */
  if ( this->partialMatch.has_value() && this->partialNameMatch.has_value() )
    {
      throw InvalidPkgQueryArg( "'partialmatch' and 'partialNameMatch' filters "
                                "may not be used together." );
    }
}

/* -------------------------------------------------------------------------- */

void
to_json( nlohmann::json & jto, const PkgQueryArgs & args )
{
  jto = {
    { "name", args.name },
    { "pname", args.pname },
    { "version", args.version },
    { "semver", args.semver },
    { "partialMatch", args.partialMatch },
    { "partialNameMatch", args.partialNameMatch },
    { "pnameOrAttrName", args.pnameOrAttrName },
    { "licenses", args.licenses },
    { "allowBroken", args.allowBroken },
    { "allowUnfree", args.allowUnfree },
    { "preferPreReleases", args.preferPreReleases },
    { "subtrees", args.subtrees },
    { "systems", args.systems },
    { "relPath", args.relPath },
    { "limit", args.limit },
    { "deduplicate", args.deduplicate },
  };
}

/* -------------------------------------------------------------------------- */

void
PkgQueryArgs::clear()
{
  this->name              = std::nullopt;
  this->pname             = std::nullopt;
  this->version           = std::nullopt;
  this->semver            = std::nullopt;
  this->partialMatch      = std::nullopt;
  this->partialNameMatch  = std::nullopt;
  this->pnameOrAttrName   = std::nullopt;
  this->licenses          = std::nullopt;
  this->allowBroken       = false;
  this->allowUnfree       = true;
  this->preferPreReleases = false;
  this->subtrees          = std::nullopt;
  this->systems           = { nix::settings.thisSystem.get() };
  this->relPath           = std::nullopt;
}


/* -------------------------------------------------------------------------- */

void
PkgQuery::addSelection( std::string_view column )
{
  if ( this->firstSelect ) { this->firstSelect = false; }
  else { this->selects << ", "; }
  this->selects << column;
}

void
PkgQuery::addOrderBy( std::string_view order )
{
  if ( this->firstOrder ) { this->firstOrder = false; }
  else { this->orders << ", "; }
  this->orders << order;
}

void
PkgQuery::addWhere( std::string_view cond )
{
  if ( this->firstWhere ) { this->firstWhere = false; }
  else { this->wheres << " AND "; }
  this->wheres << "( " << cond << " )";
}


/* -------------------------------------------------------------------------- */

void
PkgQuery::clearBuilt()
{
  this->selects.clear();
  this->orders.clear();
  this->wheres.clear();
  this->firstSelect = true;
  this->firstOrder  = true;
  this->firstWhere  = true;
  this->binds       = {};
}


/* -------------------------------------------------------------------------- */

static void
addIn( std::stringstream & oss, const std::vector<std::string> & elems )
{
  oss << " IN ( ";
  bool first = true;
  for ( const auto & elem : elems )
    {
      if ( first ) { first = false; }
      else { oss << ", "; }
      oss << '\'' << elem << '\'';
    }
  oss << " )";
}


/* -------------------------------------------------------------------------- */
std::string
PkgQuery::mkPatternString( const std::string & matchString )
{
  // SQLite allows _ and % characters in pattern matching so these need to be
  // escaped, and patterns used for LIKE are surrounded with %
  std::string pattern
    = "%" + std::regex_replace( matchString, std::regex( "([_%])" ), "\\$&" )
      + "%";
  return pattern;
}

void
PkgQuery::initMatch()
{
  /* Filter by exact matches on `pname' or `attrName'. */
  if ( this->pnameOrAttrName.has_value()
       && ( ! this->pnameOrAttrName->empty() ) )
    {
      this->addSelection( "( :pnameOrAttrName = pname ) AS exactPname" );
      this->addSelection( "( :pnameOrAttrName = attrName ) AS exactAttrName" );
      binds.emplace( ":pnameOrAttrName", *this->pnameOrAttrName );
      this->addWhere( "( exactPname OR exactAttrName )" );
    }
  else
    {
      /* Add bogus `match*` values so that later `ORDER BY` works. */
      this->addSelection( "NULL AS exactPname" );
      this->addSelection( "NULL AS exactAttrName" );
    }

  /* Filter by partial matches on `pname' or `attrName'. */
  bool hasPartialNameMatch = this->partialNameMatch.has_value()
                             && ( ! this->partialNameMatch->empty() );
  /* `partialMatch' also includes matches on `description'. */
  bool hasPartialMatch
    = this->partialMatch.has_value() && ( ! this->partialMatch->empty() );
  /* `partialNameOrRelPathMatch' also includes matches on `relPath' */
  bool hasPartialNameOrRelPathMatch
    = this->partialNameOrRelPathMatch.has_value()
      && ( ! this->partialNameOrRelPathMatch->empty() );

  if ( ! ( hasPartialNameMatch || hasPartialMatch
           || hasPartialNameOrRelPathMatch ) )
    {
      /* Add bogus `match*` values so that later `ORDER BY` works. */
      this->addSelection( "NULL AS matchExactPname" );
      this->addSelection( "NULL AS matchExactAttrName" );
      this->addSelection( "NULL AS matchPartialPname" );
      this->addSelection( "NULL AS matchPartialAttrName" );
      this->addSelection( "NULL AS matchPartialDescription" );
      this->addSelection( "NULL AS matchExactRelPath" );
      this->addSelection( "NULL AS matchPartialRelPath" );
    }
  else
    {
      /* All match fields check pname and attrName. We check for exact and
       * partial matches to improve ordering. A match for attrName will also
       * match relPath, but we check attrName no matter what for ordering. */
      /* We have to add '%' around `:match' because they were added for
       * use with `LIKE'. */
      this->addSelection(
        "LOWER( pname ) = LOWER( :partialMatch ) AS matchExactPname" );
      this->addSelection(
        "LOWER( attrName ) = LOWER( :partialMatch ) AS matchExactAttrName" );
      this->addSelection( "( pname LIKE :partialMatchPattern ESCAPE '\\' ) AS "
                          "matchPartialPname" );
      this->addSelection( "( attrName LIKE :partialMatchPattern ESCAPE '\\' ) "
                          "AS matchPartialAttrName" );

      if ( hasPartialNameMatch )
        {
          /* Add `%` before binding so `LIKE` works. */
          binds.emplace( ":partialMatch", *this->partialNameMatch );
          binds.emplace( ":partialMatchPattern",
                         mkPatternString( *this->partialNameMatch ) );
          this->addWhere( "( matchExactPname OR matchExactAttrName OR"
                          "  matchPartialPname OR matchPartialAttrName"
                          ")" );
        }

      if ( hasPartialMatch )
        {
          this->addSelection(
            "( description LIKE :partialMatchPattern ESCAPE '\\' ) AS "
            "matchPartialDescription" );
          /* Add `%` before binding so `LIKE` works. */
          binds.emplace( ":partialMatch", *this->partialMatch );
          binds.emplace( ":partialMatchPattern",
                         mkPatternString( *this->partialMatch ) );
          this->addWhere( "( matchExactPname OR matchExactAttrName OR"
                          "  matchPartialPname OR matchPartialAttrName OR"
                          "  matchPartialDescription "
                          ")" );
        }
      else { this->addSelection( "NULL AS matchPartialDescription" ); }

      if ( hasPartialNameOrRelPathMatch )
        {
          /* Join relPath with '.' so searches can include dots. */
          this->addSelection( "(SELECT LOWER( group_concat(value, '.') ) "
                              "= LOWER( :partialMatch )"
                              "FROM json_each(v_PackagesSearch.relPath)) AS "
                              "matchExactRelPath" );
          this->addSelection( "(SELECT group_concat(value, '.') LIKE "
                              ":partialMatchPattern ESCAPE '\\' "
                              "FROM json_each(v_PackagesSearch.relPath)) AS "
                              "matchPartialRelPath" );
          /* Add `%` before binding so `LIKE` works. */
          binds.emplace( ":partialMatch",
                         ( *this->partialNameOrRelPathMatch ) );
          binds.emplace( ":partialMatchPattern",
                         mkPatternString( *this->partialNameOrRelPathMatch ) );
          this->addWhere( "( matchExactPname OR matchExactAttrName OR"
                          "  matchPartialPname OR matchPartialAttrName OR"
                          "  matchPartialRelPath"
                          ")" );
        }
      else
        {
          this->addSelection( "NULL AS matchExactRelPath" );
          this->addSelection( "NULL AS matchPartialRelPath" );
        }
    }
}


/* -------------------------------------------------------------------------- */

void
PkgQuery::initSubtrees()
{
  /* Handle `subtrees' filtering. */
  if ( this->subtrees.has_value() && ( ! this->subtrees->empty() ) )
    {
      size_t                   idx = 0;
      std::vector<std::string> lst;
      std::stringstream        rank;
      rank << "CASE ";
      for ( const auto subtree : *this->subtrees )
        {
          lst.emplace_back( to_string( subtree ) );
          rank << "WHEN subtree = '" << lst.back() << "' THEN " << idx << " ";
          ++idx;
        }
      /* Wrap up rankings assignment. */
      rank << "END AS subtreesRank";
      this->addSelection( rank.str() );
      /* subtree IN ( ...  ) */
      std::stringstream cond;
      cond << "subtree";
      addIn( cond, lst );
      this->addWhere( cond.str() );
    }
  else
    {
      /* Add bogus rank so `ORDER BY subtreesRank' works. */
      this->addSelection( "0 AS subtreesRank" );
    }
}


/* -------------------------------------------------------------------------- */

void
PkgQuery::initSystems()
{
  /* Handle `systems' filtering. */
  {
    std::stringstream cond;
    cond << "system";
    addIn( cond, this->systems );
    this->addWhere( cond.str() );
  }
  if ( ! this->systems.empty() )
    {
      size_t            idx = 0;
      std::stringstream rank;
      rank << "CASE ";
      for ( const auto & system : this->systems )
        {
          rank << "WHEN system = '" << system << "' THEN " << idx << " ";
          ++idx;
        }
      rank << "END AS systemsRank";
      this->addSelection( rank.str() );
    }
  else
    {
      /* Add a bogus rank to `ORDER BY systemsRank' works. */
      this->addSelection( "0 AS systemsRank" );
    }
}


/* -------------------------------------------------------------------------- */

void
PkgQuery::initOrderBy()
{
  /* Establish ordering. */
  this->addOrderBy( R"SQL(
    exactPname              DESC
  , matchExactPname         DESC
  , exactAttrName           DESC
  , matchExactAttrName      DESC
  , matchExactRelPath       DESC
  , depth                   ASC
  , matchPartialPname       DESC
  , matchPartialAttrName    DESC
  , matchPartialRelPath     DESC
  , matchPartialDescription DESC

  , subtreesRank ASC
  , systemsRank ASC
  , pname ASC
  , versionType ASC
  )SQL" );

  /* Handle `preferPreReleases' and semver parts. */
  if ( this->preferPreReleases )
    {
      this->addOrderBy( R"SQL(
        major  DESC NULLS LAST
      , minor  DESC NULLS LAST
      , patch  DESC NULLS LAST
      , preTag DESC NULLS FIRST
      )SQL" );
    }
  else
    {
      this->addOrderBy( R"SQL(
        preTag DESC NULLS FIRST
      , major  DESC NULLS LAST
      , minor  DESC NULLS LAST
      , patch  DESC NULLS LAST
      )SQL" );
    }

  this->addOrderBy( R"SQL(
    versionDate DESC NULLS LAST
  -- Lexicographic as fallback for misc. versions
  , v_PackagesSearch.version ASC NULLS LAST
  , brokenRank ASC
  , unfreeRank ASC
  , attrName ASC
  )SQL" );
}


/* -------------------------------------------------------------------------- */

void
PkgQuery::init()
{
  this->clearBuilt();

  /* Validate parameters */
  this->check();

  this->addSelection( "*" );

  /* Handle fuzzy matching filtering. */
  this->initMatch();

  /* Handle `pname' filtering. */
  if ( this->name.has_value() )
    {
      this->addWhere( "name = :name" );
      this->binds.emplace( ":name", *this->name );
    }

  /* Handle `pname' filtering. */
  if ( this->pname.has_value() )
    {
      this->addWhere( "pname = :pname" );
      this->binds.emplace( ":pname", *this->pname );
    }

  /* Handle `version' and `semver' filtering.  */
  if ( this->version.has_value() )
    {
      this->addWhere( "version = :version" );
      this->binds.emplace( ":version", *this->version );
    }
  else if ( this->semver.has_value() )
    {
      this->addWhere( "semver IS NOT NULL" );
    }

  /* Handle `licenses' filtering. */
  if ( this->licenses.has_value() && ( ! this->licenses->empty() ) )
    {
      this->addWhere( "license IS NOT NULL" );
      /* licenses IN ( ... ) */
      std::stringstream cond;
      cond << "license";
      addIn( cond, *this->licenses );
      this->addWhere( cond.str() );
    }

  /* Handle `broken' filtering. */
  if ( ! this->allowBroken )
    {
      this->addWhere( "( broken IS NULL ) OR ( broken = FALSE )" );
    }

  /* Handle `unfree' filtering. */
  if ( ! this->allowUnfree )
    {
      this->addWhere( "( unfree IS NULL ) OR ( unfree = FALSE )" );
    }

  /* Handle `relPath' filtering */
  if ( this->relPath.has_value() )
    {
      this->addWhere( "relPath = :relPath" );
      nlohmann::json relPath = *this->relPath;
      this->binds.emplace( ":relPath", relPath.dump() );
    }

  this->initSubtrees();
  this->initSystems();
  this->initOrderBy();
}


/* -------------------------------------------------------------------------- */

std::string
PkgQuery::str() const
{
  std::stringstream qry;
  qry << "SELECT ";
  bool firstExport = true;
  for ( const auto & column : this->exportedColumns )
    {
      if ( firstExport ) { firstExport = false; }
      else { qry << ", "; }
      qry << column;
    }
  qry << " FROM ( SELECT ";
  if ( this->firstSelect ) { qry << "*"; }
  else { qry << this->selects.str(); }
  qry << " FROM v_PackagesSearch";
  if ( ! this->firstWhere ) { qry << " WHERE " << this->wheres.str(); }
  /* This will cause an arbitrary row to be chosen for all values other than
   * relPath. See "a single arbitrarily chosen row from within the group" from
   * https://www.sqlite.org/lang_select.html. This is a bit hacky, but we know
   * that `flox search` only uses `relPath` and `description`, and we assume
   * that `description` is the same for all packages that share `relPath`. */
  if ( this->deduplicate ) { qry << "\n GROUP BY relPath\n"; }
  if ( ! this->firstOrder ) { qry << " ORDER BY " << this->orders.str(); }
  qry << " )";
  // Dump the bindings as well
  if ( ! this->binds.empty() )
    {
      qry << '\n' << "-- ... with bindings:" << '\n';
      for ( const auto & bind : this->binds )
        {
          qry << "-- " << bind.first << " : " << bind.second << '\n';
        }
    }

  return qry.str();
}


/* -------------------------------------------------------------------------- */

std::unordered_set<std::string>
PkgQuery::filterSemvers(
  const std::unordered_set<std::string> & versions ) const
{
  static const std::vector<std::string> ignores
    = { "", "*", "any", "^*", "~*", "x", "X" };
  if ( ( ! this->semver.has_value() )
       || ( std::find( ignores.begin(), ignores.end(), *this->semver )
            != ignores.end() ) )
    {
      return versions;
    }
  std::list<std::string>          args( versions.begin(), versions.end() );
  std::unordered_set<std::string> rsl;
  for ( auto & version : versions::semverSat( *this->semver, args ) )
    {
      rsl.emplace( std::move( version ) );
    }
  return rsl;
}


/* -------------------------------------------------------------------------- */

std::shared_ptr<sqlite3pp::query>
PkgQuery::bind( sqlite3pp::database & pdb ) const
{
  std::string                       stmt = this->str();
  std::shared_ptr<sqlite3pp::query> qry
    = std::make_shared<sqlite3pp::query>( pdb, stmt.c_str() );
  for ( const auto & [var, val] : this->binds )
    {
      qry->bind( var.c_str(), val, sqlite3pp::copy );
    }
  return qry;
}


/* -------------------------------------------------------------------------- */

std::vector<row_id>
PkgQuery::execute( sqlite3pp::database & pdb ) const
{
  std::shared_ptr<sqlite3pp::query> qry = this->bind( pdb );
  std::vector<row_id>               rsl;

  /* If we don't need to handle `semver' this is easy. */
  if ( ! this->semver.has_value() )
    {
      for ( const auto & row : *qry )
        {
          rsl.push_back( row.get<long long>( 0 ) );
        }
      return rsl;
    }

  /* We can handle quite a bit of filtering and ordering in SQL, but `semver`
   * has to be handled with post-processing here. */

  std::unordered_set<std::string> versions;
  /* Use a vector to preserve ordering original ordering. */
  std::vector<std::pair<row_id, std::string>> idVersions;
  for ( const auto & row : *qry )
    {
      const auto & [_, version]
        = idVersions.emplace_back( row.get<long long>( 0 ),
                                   row.get<std::string>( 1 ) );
      versions.emplace( version );
    }
  versions = this->filterSemvers( versions );
  /* Filter SQL results to be those in the satisfactory list. */
  for ( const auto & elem : idVersions )
    {
      if ( versions.find( elem.second ) != versions.end() )
        {
          rsl.push_back( elem.first );
        }
    }
  return rsl;
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
