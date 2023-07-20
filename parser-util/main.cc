/* ========================================================================== *
 *
 * Parse various URIs and junk using `nix' libraries and expose them in a
 * trivially simple way so that they can be used consumed by other software.
 *
 * -------------------------------------------------------------------------- */

#include <stddef.h>
#include <iostream>
#include <string>
#include <cstring>
#include <nlohmann/json.hpp>
#include <nix/shared.hh>
#include <nix/eval.hh>
#include <nix/eval-inline.hh>
#include <nix/flake/flake.hh>
#include <nix/store-api.hh>
#include <cassert>
#include <ranges>


/* -------------------------------------------------------------------------- */

static nix::flake::LockFlags floxFlakeLockFlags = {
  .updateLockFile = false
, .writeLockFile  = false
, .applyNixConfig = false
};


/* -------------------------------------------------------------------------- */

namespace nix {
    void
  to_json( nlohmann::json & j, const FlakeRef & ref )
  {
    j = { { "string", ref.to_string() }
        , { "attrs",  fetchers::attrsToJSON( ref.toAttrs() ) }
        };
  }
}  /* End namespace `nix' */


/* -------------------------------------------------------------------------- */

  static nlohmann::json
parseURI( const char * arg )
{
  try
    {
      nix::ParsedURL       url    = nix::parseURL( arg );
      nix::ParsedUrlScheme scheme = nix::parseUrlScheme( url.scheme );

      nlohmann::json schemeJSON = {
        { "full",        url.scheme       }
      , { "application", nlohmann::json() }
      , { "transport",   scheme.transport }
      };
      if ( scheme.application.has_value() )
        {
          schemeJSON["application"] = scheme.application.value();
        }

      nlohmann::json uriJSON = {
        { "base",      std::move( url.base )     }
      , { "scheme",    std::move( schemeJSON )   }
      , { "authority", nlohmann::json()          }
      , { "path",      std::move( url.path )     }
      , { "fragment",  std::move( url.fragment ) }
      , { "query",     std::move( url.query )    }
      };
      if ( url.authority.has_value() )
        {
          uriJSON["authority"] = url.authority.value();
        }

      return uriJSON;
    }
  catch( std::exception & e )
    {
      std::cerr << e.what() << std::endl;
      exit( EXIT_FAILURE );
    }

  /* Unreachable */
  assert( false );
  return nlohmann::json();
}


/* -------------------------------------------------------------------------- */

  static nlohmann::json
parseAndResolveRef( nix::EvalState & state, const char * arg )
{
  bool isJSONArg = strchr( arg, '{' ) != nullptr;

  nlohmann::json rawInput = isJSONArg ? nlohmann::json::parse( arg ) : arg;

  try
    {
      nix::FlakeRef originalRef =
        isJSONArg ? nix::FlakeRef::fromAttrs(
                      nix::fetchers::jsonToAttrs( rawInput )
                    )
                  : nix::parseFlakeRef( arg, nix::absPath( "." ), true, false );

      try
        {
          nix::FlakeRef resolvedRef = originalRef.resolve( state.store );
          return {
            { "input",       std::move( rawInput ) }
          , { "originalRef", originalRef           }
          , { "resolvedRef", resolvedRef           }
          };
        }
      catch( ... )
        {
          return {
            { "input",       std::move( rawInput ) }
          , { "originalRef", originalRef           }
          , { "resolvedRef", nlohmann::json()      }
          };
        }
    }
  catch( std::exception & e )
    {
      std::cerr << e.what() << std::endl;
      exit( EXIT_FAILURE );
    }

  /* Unreachable */
  assert( false );
  return nlohmann::json();
}


/* -------------------------------------------------------------------------- */

/* Essentially similar to `parseAndResolveRef' but also emits `lockedRef'.
 * The reason to have two separate functions is to avoid fetching in cases where
 * the user strictly wants to parse/resolve. */
  nlohmann::json
lockFlake( nix::EvalState & state, const char * arg )
{
  bool isJSONArg = strchr( arg, '{' ) != nullptr;

  nlohmann::json rawInput =
    isJSONArg ? nlohmann::json::parse( arg ) : arg;

  try
    {
      nix::FlakeRef originalRef =
        isJSONArg ? nix::FlakeRef::fromAttrs(
                      nix::fetchers::jsonToAttrs( rawInput )
                    )
                  : nix::parseFlakeRef( arg, nix::absPath( "." ) );

      nix::flake::LockedFlake locked = nix::flake::lockFlake(
        state
      , originalRef
      , floxFlakeLockFlags
      );

      return {
        { "input",       std::move( rawInput )    }
      , { "originalRef", locked.flake.originalRef }
      , { "resolvedRef", locked.flake.resolvedRef }
      , { "lockedRef",   locked.flake.lockedRef   }
      };
    }
  catch( std::exception & e )
    {
      std::cerr << e.what() << std::endl;
      exit( EXIT_FAILURE );
    }

  /* Unreachable */
  assert( false );
  return nlohmann::json();
}


/* -------------------------------------------------------------------------- */

  static nlohmann::json
parseInstallable( nix::EvalState & state, const char * arg )
{
  try
    {
      std::tuple<nix::FlakeRef, std::string, nix::ExtendedOutputsSpec> parsed =
        nix::parseFlakeRefWithFragmentAndExtendedOutputsSpec(
          std::string( arg )
        , nix::absPath( "." )
        );

      nix::FlakeRef            ref     = std::get<0>( parsed );
      nix::ExtendedOutputsSpec exOuts  = std::get<2>( parsed );
      nlohmann::json           outputs;

      if ( std::holds_alternative<nix::OutputsSpec>( exOuts.raw() ) )
        {
          nix::OutputsSpec outSpec = std::get<nix::OutputsSpec>( exOuts.raw() );
          if ( std::holds_alternative<nix::OutputNames>( outSpec.raw() ) )
            {
              nix::OutputNames outs =
                std::get<nix::OutputNames>( outSpec.raw() );
              for ( auto & out : outs )
                {
                  outputs.push_back( std::move( out ) );
                }
            }
          else  /* All */
            {
              outputs = "all";
            }
        }
      else
        {
          outputs = "default";
        }

      return {
        { "input",    std::move( arg ) }
      , { "ref",      ref              }
      , { "attrPath",
          nix::tokenizeString<std::vector<std::string>>(
            std::get<1>( parsed )
          )
        }
      , { "outputs", std::move( outputs ) }
      };
    }
  catch( std::exception & e )
    {
      // TODO: Catch errors here and make the messages prettier before printing.
      std::cerr << e.what() << std::endl;
      exit( EXIT_FAILURE );
    }

  /* Unreachable */
  assert( false );
  return nlohmann::json();
}


/* -------------------------------------------------------------------------- */

static const char usageMsg[] =
    "Usage: parser-util [-r|-l|-i|-u] <URI|JSON-ATTRS>\n"
    "Usage: parser-util <-h|--help|--usage>";


  int
main( int argc, char * argv[], char ** envp )
{
  nix::initNix();
  nix::initGC();

  nix::evalSettings.pureEval = false;

  nix::EvalState state( {}, nix::openStore() );

  char   cmd = '\0';
  char * arg = nullptr;

  nlohmann::json j;

  if ( argc < 2 )
    {
      std::cerr << "Too few arguments!" << std::endl
                << usageMsg << std::endl;
      return EXIT_FAILURE;
    }

  if ( ( argv[1][0] == '-' ) && ( std::string_view( argv[1] ).size() < 2 ) )
    {
      std::cerr << "Unrecognized command flag: " << argv[1] << std::endl
                << usageMsg << std::endl;
      return EXIT_FAILURE;
    }

  if ( std::string_view( argv[1] ) == "--usage"  )
    {
      std::cout << usageMsg << std::endl;
      return EXIT_SUCCESS;
    }

  if ( ( std::string_view( argv[1] ) == "--help"  ) || ( argv[1][1] == 'h' ) )
    {
      cmd = 'h';
    }
  else
    {
      switch ( argc )
        {
          case 2:
            arg = argv[1];
            /* Guess between `parseAndResolveRef' vs. `parseInstallable' */
            if ( strchr( argv[1], '#' ) != nullptr ) { cmd = 'i'; }
            else                                     { cmd = 'r'; }
            break;

          case 3:
            cmd = argv[1][1];
            arg = argv[2];
            break;

          default:
            std::cerr << "Too many arguments!" << std::endl
                      << usageMsg << std::endl;
            return EXIT_FAILURE;
            break;
        }
    }

  switch ( cmd )
    {
      case 'r': j = parseAndResolveRef( state, arg ); break;
      case 'l': j = lockFlake(          state, arg ); break;
      case 'i': j = parseInstallable(   state, arg ); break;
      case 'u': j = parseURI(                  arg ); break;
      case 'h':
        std::cout << usageMsg << std::endl << std::endl
                  << "Options:" << std::endl
                  << "  -r <FLAKE-URI|JSON>  parseAndResolveRef" << std::endl
                  << "  -l <FLAKE-URI|JSON>  lockFlake"          << std::endl
                  << "  -i INSTALLABLE-URI   parseAndResolveRef" << std::endl
                  << "  -u URI               parseURI"           << std::endl
                  << "     --usage           show usage message" << std::endl
                  << "  -h,--help            show this message"  << std::endl;
        return EXIT_SUCCESS;
        break;

      default:
        std::cerr << "Unrecognized command flag: " << argv[1] << std::endl
                  << usageMsg << std::endl;

        return EXIT_FAILURE;
        break;
    }


  std::cout << j.dump() << std::endl;

  return EXIT_SUCCESS;
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
