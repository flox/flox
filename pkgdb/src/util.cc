/* ========================================================================== *
 *
 * @file flox/util.cc
 *
 * @brief Miscellaneous helper functions.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <cctype>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <initializer_list>
#include <string>
#include <string_view>
#include <vector>

#include <nix/attrs.hh>
#include <nix/fetchers.hh>

#include <nlohmann/json.hpp>

#include "flox/core/exceptions.hh"
#include "flox/core/types.hh"
#include "flox/core/util.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

bool
isSQLiteDb( const std::string & dbPath )
{
  std::filesystem::path path( dbPath );
  if ( ! std::filesystem::exists( path ) ) { return false; }
  if ( std::filesystem::is_directory( path ) ) { return false; }

  /* Read file magic */
  static const char expectedMagic[16] = "SQLite format 3";  // NOLINT

  char buffer[16];  // NOLINT
  std::memset( &buffer[0], '\0', sizeof( buffer ) );
  FILE * filep = fopen( dbPath.c_str(), "rb" );

  std::clearerr( filep );

  const size_t nread
    = std::fread( &buffer[0], sizeof( buffer[0] ), sizeof( buffer ), filep );
  if ( nread != sizeof( buffer ) )
    {
      if ( std::feof( filep ) != 0 )
        {
          std::fclose( filep );  // NOLINT
          return false;
        }
      if ( std::ferror( filep ) != 0 )
        {
          std::fclose( filep );  // NOLINT
          throw flox::FloxException( "Failed to read file " + dbPath );
        }
      std::fclose( filep );  // NOLINT
      return false;
    }
  std::fclose( filep );  // NOLINT
  return std::string_view( &buffer[0] )
         == std::string_view( &expectedMagic[0] );
}


/* -------------------------------------------------------------------------- */

nix::FlakeRef
parseFlakeRef( const std::string & flakeRef )
{
  return ( flakeRef.find( '{' ) == std::string::npos )
           ? nix::parseFlakeRef( flakeRef )
           : nix::FlakeRef::fromAttrs(
             nix::fetchers::jsonToAttrs( nlohmann::json::parse( flakeRef ) ) );
}


/* -------------------------------------------------------------------------- */

nlohmann::json
parseOrReadJSONObject( const std::string & jsonOrPath )
{
  if ( jsonOrPath.find( '{' ) != std::string::npos )
    {
      return nlohmann::json::parse( jsonOrPath );
    }
  std::ifstream jfile( jsonOrPath );
  return nlohmann::json::parse( jfile );
}


/* -------------------------------------------------------------------------- */

nlohmann::json
readAndCoerceJSON( const std::filesystem::path & path )
{
  if ( ! std::filesystem::exists( path ) )
    {
      throw flox::FloxException( "File `" + path.string()
                                 + "' does not exist" );
    }

  std::ifstream ifs( path );
  auto          ext = path.extension();
  if ( ( ext == ".json" ) || ( ext == ".lock" ) )
    {
      return nlohmann::json::parse( ifs );
    }

  /* Read file to buffer */
  std::ostringstream oss;
  if ( ( ext == ".yaml" ) || ( ext == ".yml" ) )
    {
      oss << ifs.rdbuf();
      return yamlToJSON( oss.str() );
    }
  else if ( ext == ".toml" )
    {
      oss << ifs.rdbuf();
      return tomlToJSON( oss.str() );
    }
  else
    {
      throw flox::FloxException( "Cannot convert file extension `"
                                 + ext.string() + "' to JSON" );
    }
}


/* -------------------------------------------------------------------------- */

std::vector<std::string>
splitAttrPath( std::string_view path )
{
  std::vector<std::string> parts;

  bool inSingleQuote = false;
  bool inDoubleQuote = false;
  bool wasEscaped    = false;
  auto start         = path.begin();

  /* Remove outer quotes and unescape. */
  auto dequote = [&]( const std::string & part ) -> std::string
  {
    auto itr = part.begin();
    auto end = part.end();

    /* Remove outer quotes. */
    if ( ( ( part.front() == '\'' ) && ( part.back() == '\'' ) )
         || ( ( part.front() == '"' ) && ( part.back() == '"' ) ) )
      {
        ++itr;
        --end;
      }

    /* Remove escape characters. */
    std::string rsl;
    bool        wasEscaped = false;
    for ( ; itr != end; ++itr )
      {
        if ( wasEscaped ) { wasEscaped = false; }
        else if ( ( *itr ) == '\\' )
          {
            wasEscaped = true;
            continue;
          }
        rsl.push_back( *itr );
      }

    return rsl;
  }; /* End lambda `dequote' */

  /* Split by dots, handling quotes. */
  for ( auto itr = path.begin(); itr != path.end(); ++itr )
    {
      if ( wasEscaped ) { wasEscaped = false; }
      else if ( ( *itr ) == '\\' ) { wasEscaped = true; }
      else if ( ( ( *itr ) == '\'' ) && ( ! inDoubleQuote ) )
        {
          inSingleQuote = ! inSingleQuote;
        }
      else if ( ( ( *itr ) == '"' ) && ( ! inSingleQuote ) )
        {
          inDoubleQuote = ! inDoubleQuote;
        }
      else if ( *itr == '.' && ( ! inSingleQuote ) && ( ! inDoubleQuote ) )
        {
          parts.emplace_back( dequote( std::string( start, itr ) ) );
          start = itr + 1;
        }
    }

  if ( start != path.end() )
    {
      parts.emplace_back( dequote( std::string( start, path.end() ) ) );
    }

  return parts;
}


/* -------------------------------------------------------------------------- */

bool
isUInt( std::string_view str )
{
  return ( ! str.empty() )
         && ( std::find_if( str.begin(),
                            str.end(),
                            []( unsigned char chr )
                            { return std::isdigit( chr ) == 0; } )
              == str.end() );
}


/* -------------------------------------------------------------------------- */

bool
hasPrefix( std::string_view prefix, std::string_view str )
{
  if ( str.size() < prefix.size() ) { return false; }
  return str.find( prefix ) == 0;
}


/* -------------------------------------------------------------------------- */

std::string &
ltrim( std::string & str )
{
  str.erase( str.begin(),
             std::find_if( str.begin(),
                           str.end(),
                           []( unsigned char chr )
                           { return ! std::isspace( chr ); } ) );
  return str;
}

std::string &
rtrim( std::string & str )
{
  str.erase( std::find_if( str.rbegin(),
                           str.rend(),
                           []( unsigned char chr )
                           { return ! std::isspace( chr ); } )
               .base(),
             str.end() );
  return str;
}

std::string &
trim( std::string & str )
{
  rtrim( str );
  ltrim( str );
  return str;
}


std::string
ltrim_copy( std::string_view str )
{
  std::string rsl( str );
  ltrim( rsl );
  return rsl;
}

std::string
rtrim_copy( std::string_view str )
{
  std::string rsl( str );
  rtrim( rsl );
  return rsl;
}

std::string
trim_copy( std::string_view str )
{
  std::string rsl( str );
  trim( rsl );
  return rsl;
}


/* -------------------------------------------------------------------------- */

std::string
extract_json_errmsg( nlohmann::json::exception & err )
{
  /* All of the nlohmann::json::exception messages are formatted like so:
   * [something] actually useful message. */
  std::string            full( err.what() );
  std::string::size_type idx = full.find( "]" );
  idx += 1; /* Don't include the leading space */
  std::string userFriendly = full.substr( idx, full.size() );
  return userFriendly;
}


/* -------------------------------------------------------------------------- */

std::string
displayableGlobbedPath( const flox::AttrPathGlob & attrs )
{
  std::vector<std::string> globbed;
  for ( const std::optional<std::string> & attr : attrs )
    {
      if ( attr.has_value() ) { globbed.emplace_back( *attr ); }
      else { globbed.emplace_back( "*" ); }
    }
  auto fold
    = []( std::string a, std::string b ) { return std::move( a ) + '.' + b; };

  std::string s = std::accumulate( std::next( globbed.begin() ),
                                   globbed.end(),
                                   globbed[0],
                                   fold );
  return s;
}


/* -------------------------------------------------------------------------- */

void
printLog( const nix::Verbosity & lvl, const std::string & msg )
{
  nix::logger->log( lvl, msg );
}

void
traceLog( const std::string & msg )
{
  printLog( nix::Verbosity::lvlVomit, msg );
}

void
debugLog( const std::string & msg )
{
  printLog( nix::Verbosity::lvlDebug, msg );
}

void
infoLog( const std::string & msg )
{
  printLog( nix::Verbosity::lvlInfo, msg );
}

void
warningLog( const std::string & msg )
{
  printLog( nix::Verbosity::lvlWarn, msg );
}

void
errorLog( const std::string & msg )
{
  printLog( nix::Verbosity::lvlError, msg );
}


/* -------------------------------------------------------------------------- */

bool
isNixpkgsRef( nix::FlakeRef const & ref )
{
  if ( ref.input.getType() != "github" ) { return false; }
  try
    {
      if ( auto owner
           = nix::fetchers::maybeGetStrAttr( ref.input.attrs, "owner" ).value();
           owner != "NixOS" )
        {
          return false;
        }
    }
  catch ( nix::Error & err )
    {
      return false;
    }
  try
    {
      if ( auto owner
           = nix::fetchers::maybeGetStrAttr( ref.input.attrs, "repo" ).value();
           owner != "nixpkgs" )
        {
          return false;
        }
    }
  catch ( nix::Error & err )
    {
      return false;
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
