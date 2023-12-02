/* ========================================================================== *
 *
 * @file versions.cc
 *
 * @brief Interfaces used to perform version number analysis, especially
 *        _Semantic Version_ processing.
 *
 *
 * -------------------------------------------------------------------------- */

#include <istream>
#include <list>
#include <map>
#include <optional>
#include <regex>
#include <string>
#include <string_view>
#include <sys/types.h>
#include <utility>
#include <vector>

#include <nix/types.hh>
#include <nix/util.hh>

#include "versions.hh"


/* -------------------------------------------------------------------------- */

/** Interfaces for analyzing version numbers */
namespace versions {

/* -------------------------------------------------------------------------- */

/** Matches Semantic Version strings, e.g. `4.2.0-pre'. */
static const char * const semverREStr
  = "(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)(-[-[:alnum:]_+.]+)?";

/** Matches _loose_ versions which may omit trailing 0s. */
static const char * const semverLooseREStr
  = "(0|[1-9][0-9]*)(\\.(0|[1-9][0-9]*)(\\.(0|[1-9][0-9]*))?)?"
    "(-[-[:alnum:]_+.]+)?";

/** Coercively matches Semantic Version Strings, e.g. `v1.0-pre'. */
static const char * const semverCoerceREStr
  = "(.*@)?[vV]?(0*([0-9]+)(\\.0*([0-9]+)(\\.0*([0-9]+))?)?(-[-[:alnum:]_+.]+)?"
    ")";

/** Match '-' separated date strings, e.g. `2023-05-31' or `5-1-23'. */
static const char * const dateREStr
  = "([12][0-9][0-9][0-9]-[0-1]?[0-9]-[0-3]?[0-9]|" /* Y-M-D */
    "[0-1]?[0-9]-[0-3]?[0-9]-[12][0-9][0-9][0-9])"  /* M-D-Y */
    "(-[-[:alnum:]_+.]+)?";


/* -------------------------------------------------------------------------- */

bool
isSemver( const std::string & version )
{
  static const std::regex semverRE( semverREStr, std::regex::ECMAScript );
  return std::regex_match( version, semverRE );
}


/* -------------------------------------------------------------------------- */

bool
isDate( const std::string & version )
{
  static const std::regex dateRE( dateREStr, std::regex::ECMAScript );
  return std::regex_match( version, dateRE );
}


/* -------------------------------------------------------------------------- */

bool
isCoercibleToSemver( const std::string & version )
{
  static const std::regex dateRE( dateREStr, std::regex::ECMAScript );
  static const std::regex semverCoerceRE( semverCoerceREStr,
                                          std::regex::ECMAScript );
  return ( ! std::regex_match( version, dateRE ) )
         && std::regex_match( version, semverCoerceRE );
}


/* -------------------------------------------------------------------------- */

std::optional<std::string>
coerceSemver( std::string_view version )
{
  static const std::regex semverRE( semverREStr, std::regex::ECMAScript );
  static const std::regex semverCoerceRE( semverCoerceREStr,
                                          std::regex::ECMAScript );
  std::string             vsn( version );
  /* If it's already a match for a proper semver we're done. */
  if ( std::regex_match( vsn, semverRE ) ) { return { vsn }; }

  /* Try try matching the coercive pattern. */
  std::smatch match;
  if ( isDate( vsn ) || ( ! std::regex_match( vsn, match, semverCoerceRE ) ) )
    {
      return std::nullopt;
    }

  /**
   * Capture Groups Example:
   *   [0]: foo@v1.02.0-pre
   *   [1]: foo@
   *   [2]: 1.02.0-pre
   *   [3]: 1
   *   [4]: .02.0
   *   [5]: 2
   *   [6]: .0
   *   [7]: 0
   *   [8]: -pre
   */
  static const size_t majorIdx  = 3;
  static const size_t minorIdx  = 5;
  static const size_t patchIdx  = 7;
  static const size_t preTagIdx = 8;


  /* The `str()' function is destructive and works by adding null terminators to
   * the original string.
   * If we attempt to convert each submatch from left to right we will clobber
   * some characters with null terminators.
   * To avoid this we convert each submatch to a string from right to left.
   */
  std::string tag( match[preTagIdx].str() );
  std::string patch( match[patchIdx].str() );
  std::string minor( match[minorIdx].str() );

  std::string rsl( match[majorIdx].str() + "." );

  if ( minor.empty() ) { rsl += "0."; }
  else { rsl += minor + "."; }

  if ( patch.empty() ) { rsl += "0"; }
  else { rsl += patch; }

  if ( ! tag.empty() ) { rsl += tag; }

  return { rsl };
}


/* -------------------------------------------------------------------------- */

bool
isSemverRange( const std::string & range )
{
  /* Check for _modifier_ */
  static const std::string semverRangeREStr
    = "\\s*([~^><=]|>=|<=)?\\s*" + std::string( semverLooseREStr ) + ".*";
  static const std::regex semverRangeRE( semverRangeREStr,
                                         std::regex::ECMAScript );

  /* A few special tokens including the empty string are also valid. */
  static const std::regex globMatch( "\\s*(\\*|any|latest)?\\s*" );

  return std::regex_match( range, semverRangeRE )
         || std::regex_match( range, globMatch )
         || ( range.find( " - " ) != std::string::npos );
}


/* -------------------------------------------------------------------------- */

#ifndef SEMVER_PATH
#  define SEMVER_PATH "semver"
#endif

std::pair<int, std::string>
runSemver( const std::list<std::string> & args )
{
  static const std::string semverProg
    = nix::getEnv( "SEMVER" ).value_or( SEMVER_PATH );
  static const std::map<std::string, std::string> env = nix::getEnv();
  return nix::runProgram( nix::RunOptions { .program             = semverProg,
                                            .searchPath          = true,
                                            .args                = args,
                                            .uid                 = std::nullopt,
                                            .gid                 = std::nullopt,
                                            .chdir               = std::nullopt,
                                            .environment         = env,
                                            .input               = std::nullopt,
                                            .standardIn          = nullptr,
                                            .standardOut         = nullptr,
                                            .mergeStderrToStdout = false } );
}


/* -------------------------------------------------------------------------- */

/** @brief Strip any '*', 'x', or 'X' characters from the range. */
[[nodiscard]] static std::string
cleanRange( const std::string & range )
{
  std::string rsl;
  rsl.reserve( range.size() );
  for ( size_t idx = 0; idx < range.size(); ++idx )
    {
      const char chr = range[idx];
      if ( ( chr != '*' ) && ( chr != 'x' ) && ( chr != 'X' ) )
        {
          rsl.push_back( chr );
          continue;
        }
      else
        {
          /* Handle `18.x' by also dropping trailing '.'. */
          if ( rsl.back() == '.' ) { rsl.pop_back(); }
          while ( ( idx < range.size() ) && ( range[idx] != ' ' )
                  && ( range[idx] != ',' ) && ( range[idx] != '&' )
                  && ( range[idx] != '|' ) )
            {
              ++idx;
            }
          if ( idx < range.size() ) { rsl.push_back( range[idx] ); }
        }
    }
  rsl.shrink_to_fit();
  return rsl;
}


/* -------------------------------------------------------------------------- */

std::list<std::string>
semverSat( const std::string & range, const std::list<std::string> & versions )
{
  std::list<std::string> args
    = { "--include-prerelease", "--loose", "--range", cleanRange( range ) };
  for ( const auto & version : versions ) { args.push_back( version ); }
  auto [ec, lines] = runSemver( args );
  /* TODO: determine parse error vs. empty list result. */
  if ( ! nix::statusOk( ec ) ) { return {}; }
  std::list<std::string> rsl;
  std::stringstream      oss( lines );
  std::string            line;
  while ( std::getline( oss, line, '\n' ) )
    {
      if ( ! line.empty() ) { rsl.push_back( std::move( line ) ); }
    }
  return rsl;
}


/* -------------------------------------------------------------------------- */

}  // namespace versions


/* -------------------------------------------------------------------------- *
 *
 *
 * ========================================================================== */
