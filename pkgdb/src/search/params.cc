/* ========================================================================== *
 *
 * @file search/params.cc
 *
 * @brief A set of user inputs used to set input preferences and query
 *        parameters during search.
 *
 *
 * -------------------------------------------------------------------------- */

#include "flox/search/params.hh"


/* -------------------------------------------------------------------------- */

namespace flox::search {

/* -------------------------------------------------------------------------- */

void
SearchQuery::clear()
{
  this->name             = std::nullopt;
  this->pname            = std::nullopt;
  this->version          = std::nullopt;
  this->semver           = std::nullopt;
  this->partialMatch     = std::nullopt;
  this->partialNameMatch = std::nullopt;
}


/* -------------------------------------------------------------------------- */

void
SearchQuery::check() const
{
  /* `name' and `pname' or `version' cannot be used together. */
  if ( this->name.has_value() && this->pname.has_value() )
    {
      throw ParseSearchQueryException(
        "`name' and `pname' filters may not be used together." );
    }
  if ( this->name.has_value() && this->version.has_value() )
    {
      throw ParseSearchQueryException(
        "`name' and `version' filters may not be used together." );
    }

  /* `version' and `semver' cannot be used together. */
  if ( this->version.has_value() && this->semver.has_value() )
    {
      throw ParseSearchQueryException(
        "`version' and `semver' filters may not be used together." );
    }

  /* `partialMatch' and `partialNameMatch' cannot be used together. */
  if ( this->partialMatch.has_value() && this->partialNameMatch.has_value() )
    {
      throw ParseSearchQueryException(
        "`partialmatch' and `partialNameMatch' filters "
        "may not be used together." );
    }
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, SearchQuery & qry )
{
  auto getOrFail
    = [&]( const std::string & key, const nlohmann::json & from, auto & sink )
  {
    if ( from.is_null() ) { return; }
    try
      {
        from.get_to( sink );
      }
    catch ( const nlohmann::json::exception & err )
      {
        throw ParseSearchQueryException( "parsing field: 'query." + key + "'",
                                         err.what() );
      }
    catch ( ... )
      {
        throw ParseSearchQueryException( "parsing field: 'query." + key
                                         + "'." );
      }
  };

  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "name" ) { getOrFail( key, value, qry.name ); }
      else if ( key == "pname" ) { getOrFail( key, value, qry.pname ); }
      else if ( key == "version" ) { getOrFail( key, value, qry.version ); }
      else if ( key == "semver" ) { getOrFail( key, value, qry.semver ); }
      else if ( key == "match" ) { getOrFail( key, value, qry.partialMatch ); }
      else if ( key == "match-name" )
        {
          getOrFail( key, value, qry.partialNameMatch );
        }
      else if ( key == "name-match" )
        {
          throw ParseSearchQueryException(
            "unrecognized key `query.name-match' , did you "
            "mean `query.match-name'?" );
        }
      else
        {
          throw ParseSearchQueryException( "unrecognized key 'query." + key
                                           + "'." );
        }
    }
}


void
to_json( nlohmann::json & jto, const SearchQuery & qry )
{
  jto["name"]       = qry.name;
  jto["pname"]      = qry.pname;
  jto["version"]    = qry.version;
  jto["semver"]     = qry.semver;
  jto["match"]      = qry.partialMatch;
  jto["match-name"] = qry.partialNameMatch;
}


/* -------------------------------------------------------------------------- */

pkgdb::PkgQueryArgs &
SearchQuery::fillPkgQueryArgs( pkgdb::PkgQueryArgs & pqa ) const
{
  /* XXX: DOES NOT CLEAR FIRST! We are called after global preferences. */
  pqa.name             = this->name;
  pqa.pname            = this->pname;
  pqa.version          = this->version;
  pqa.semver           = this->semver;
  pqa.partialMatch     = this->partialMatch;
  pqa.partialNameMatch = this->partialNameMatch;
  return pqa;
}


/* -------------------------------------------------------------------------- */

std::optional<std::filesystem::path>
SearchParams::getLockfilePath()
{
  if ( this->lockfile.has_value()
       && std::holds_alternative<std::filesystem::path>( *this->lockfile ) )
    {
      return std::get<std::filesystem::path>( *this->lockfile );
    }
  return std::nullopt;
}


/* -------------------------------------------------------------------------- */

std::optional<resolver::LockfileRaw>
SearchParams::getLockfileRaw()
{
  if ( ! this->lockfile.has_value() ) { return std::nullopt; }
  if ( std::holds_alternative<resolver::LockfileRaw>( *this->lockfile ) )
    {
      return std::get<resolver::LockfileRaw>( *this->lockfile );
    }
  return readAndCoerceJSON(
    std::get<std::filesystem::path>( *this->lockfile ) );
}


/* -------------------------------------------------------------------------- */

std::optional<std::filesystem::path>
SearchParams::getGlobalManifestPath()
{
  if ( this->globalManifest.has_value()
       && std::holds_alternative<std::filesystem::path>(
         *this->globalManifest ) )
    {
      return std::get<std::filesystem::path>( *this->globalManifest );
    }
  return std::nullopt;
}


/* -------------------------------------------------------------------------- */

std::optional<resolver::GlobalManifestRaw>
SearchParams::getGlobalManifestRaw()
{
  if ( ! this->globalManifest.has_value() ) { return std::nullopt; }
  if ( std::holds_alternative<resolver::GlobalManifestRaw>(
         *this->globalManifest ) )
    {
      return std::get<resolver::GlobalManifestRaw>( *this->globalManifest );
    }
  return readAndCoerceJSON(
    std::get<std::filesystem::path>( *this->globalManifest ) );
}


/* -------------------------------------------------------------------------- */

std::optional<std::filesystem::path>
SearchParams::getManifestPath()
{
  if ( this->manifest.has_value()
       && std::holds_alternative<std::filesystem::path>( *this->manifest ) )
    {
      return std::get<std::filesystem::path>( *this->manifest );
    }
  return std::nullopt;
}


/* -------------------------------------------------------------------------- */

resolver::ManifestRaw
SearchParams::getManifestRaw()
{
  if ( ! this->manifest.has_value() ) { return {}; }
  if ( std::holds_alternative<resolver::ManifestRaw>( *this->manifest ) )
    {
      return std::get<resolver::ManifestRaw>( *this->manifest );
    }
  return readAndCoerceJSON(
    std::get<std::filesystem::path>( *this->manifest ) );
}


/* -------------------------------------------------------------------------- */

void
from_json( const nlohmann::json & jfrom, SearchParams & params )
{
  assertIsJSONObject<ParseSearchQueryException>( jfrom, "search query" );
  for ( const auto & [key, value] : jfrom.items() )
    {
      if ( key == "global-manifest" )
        {
          try
            {
              value.get_to( params.globalManifest );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseSearchQueryException(
                "couldn't interpret search query field `global-manifest'",
                extract_json_errmsg( e ) );
            }
        }
      else if ( key == "lockfile" )
        {
          try
            {
              value.get_to( params.lockfile );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseSearchQueryException(
                "couldn't interpret search query field `lockfile'",
                extract_json_errmsg( e ) );
            }
        }
      else if ( key == "manifest" )
        {
          try
            {
              value.get_to( params.manifest );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseSearchQueryException(
                "couldn't interpret search query field `lockfile'",
                extract_json_errmsg( e ) );
            }
        }
      else if ( key == "query" )
        {

          try
            {
              value.get_to( params.query );
            }
          catch ( nlohmann::json::exception & e )
            {
              throw ParseSearchQueryException(
                "couldn't interpret search query field `query'",
                extract_json_errmsg( e ) );
            }
        }
      else
        {
          throw ParseSearchQueryException( "unrecognized field `" + key
                                           + "' in search query" );
        }
    }
}


/* -------------------------------------------------------------------------- */

void
to_json( nlohmann::json & jto, const SearchParams & params )
{
  jto = { { "global-manifest", params.globalManifest },
          { "manifest", params.manifest },
          { "lockfile", params.lockfile },
          { "query", params.query } };
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::search


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
