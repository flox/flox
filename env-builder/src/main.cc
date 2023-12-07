/* ========================================================================== *
 *
 * @file main.cc
 *
 * @brief Executable exposing CRUD operations for package metadata.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <ifaddrs.h>
#include <netdb.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <sys/types.h>

#include <nix/command.hh>
#include <nix/common-args.hh>
#include <nix/eval.hh>
#include <nix/filetransfer.hh>
#include <nix/finally.hh>
#include <nix/globals.hh>
#include <nix/legacy.hh>
#include <nix/loggers.hh>
#include <nix/markdown.hh>
#include <nix/shared.hh>
#include <nix/store-api.hh>
#include <nlohmann/json.hpp>

#include "flox/env-builder/command.hh"


/* -------------------------------------------------------------------------- */

namespace nix {

/* -------------------------------------------------------------------------- */

/** Check if we have a non-loopback/link-local network interface. */
  static bool
haveInternet()
{
  struct ifaddrs * addrs;

  if ( getifaddrs( & addrs ) ) { return true; }

  Finally free( [&]() { freeifaddrs( addrs ); } );

  for ( auto i = addrs; i; i = i->ifa_next )
    {
      if ( ! i->ifa_addr ) { continue; }
      if ( i->ifa_addr->sa_family == AF_INET )
        {
          if ( ntohl( ( (sockaddr_in *) i->ifa_addr )->sin_addr.s_addr )
               != INADDR_LOOPBACK
             )
            {
              return true;
            }
        }
      else if ( i->ifa_addr->sa_family == AF_INET6 )
        {
          if ( ! ( IN6_IS_ADDR_LOOPBACK(
                     & ( (sockaddr_in6 *) i->ifa_addr )->sin6_addr
                   ) ||
                   IN6_IS_ADDR_LINKLOCAL(
                     & ( (sockaddr_in6 *) i->ifa_addr )->sin6_addr
                   )
                 )
             )
          {
            return true;
          }
      }
    }

  return false;
}

std::string programPath;


/* -------------------------------------------------------------------------- */

  void
mainWrapped( int argc, char * argv[] )
{
  savedArgv = argv;

  initNix();
  initGC();

#if __linux__
  if ( getuid() == 0 )
    {
      try
        {
          saveMountNamespace();
          if ( unshare( CLONE_NEWNS ) == -1 )
            {
              throw SysError( "setting up a private mount namespace" );
            }
        } catch ( Error & e ) {}
    }
#endif

  Finally f( [] { logger->stop(); } );

  programPath      = argv[0];
  auto programName = std::string( baseNameOf( programPath ) );

  evalSettings.pureEval = true;

  setLogFormat( "bar" );
  settings.verboseBuild = false;
  if ( isatty( STDERR_FILENO ) )
    {
      verbosity = lvlNotice;
    }
  else
    {
      verbosity = lvlInfo;
    }

  flox::FloxArgs args;

  if ( ( argc == 2 ) && ( std::string_view( argv[1] ) == "__dump-cli" ) )
    {
      logger->cout( args.dumpCli().dump() );
      return;
    }

  if ( ( argc == 2 ) &&
       ( std::string_view( argv[1] ) == "__dump-xp-features" )
     )
    {
      logger->cout( documentExperimentalFeatures().dump() );
      return;
    }

  Finally printCompletions( [&]()
  {
    if ( completions )
      {
        switch ( completionType )
          {
            case ctNormal:    logger->cout( "normal" );    break;
            case ctFilenames: logger->cout( "filenames" ); break;
            case ctAttrs:     logger->cout( "attrs" );     break;
          }
          for ( auto & s : * completions )
            {
              logger->cout( s.completion + "\t" + trim( s.description ) );
            }
      }
  } );

  try
    {
      args.parseCmdline( argvToStrings( argc, argv ) );
    }
  catch ( UsageError & )
    {
      if ( ! ( args.helpRequested || completions ) ) throw;
    }

  if ( args.helpRequested )
    {
      std::vector<std::string> subcommand;
      MultiCommand * command = &args;
      while ( command )
        {
          if ( command && command->command )
            {
              subcommand.push_back( command->command->first );
              command = dynamic_cast<MultiCommand *>(
                & ( * command->command->second )
              );
            }
          else
            {
              break;
            }
        }
      showHelp( subcommand, args );
      return;
    }

  if ( completions )
    {
      args.completionHook();
      return;
    }

  if ( args.showVersion )
    {
      printVersion( programName );
      return;
    }

  if ( ! args.command ) { throw UsageError( "no subcommand specified" ); }

  experimentalFeatureSettings.require(
    args.command->second->experimentalFeature()
  );

  if ( args.useNet && ( ! haveInternet() ) )
    {
      warn( "you don't have Internet access; disabling some "
            "network-dependent features"
          );
      args.useNet = false;
    }

  if ( ! args.useNet )
    {
      // FIXME: should check for command line overrides only.
      if ( ! settings.useSubstitutes.overridden )
        {
          settings.useSubstitutes = false;
        }

      if ( ! settings.tarballTtl.overridden )
        {
          settings.tarballTtl = std::numeric_limits<unsigned int>::max();
        }

      if ( ! fileTransferSettings.tries.overridden )
        {
          fileTransferSettings.tries = 0;
        }

      if ( ! fileTransferSettings.connectTimeout.overridden )
        {
          fileTransferSettings.connectTimeout = 1;
        }
    }

  if ( args.refresh )
    {
      settings.tarballTtl = 0;
      settings.ttlNegativeNarInfoCache = 0;
      settings.ttlPositiveNarInfoCache = 0;
    }

  if ( args.command->second->forceImpureByDefault() &&
       ( ! evalSettings.pureEval.overridden )
     )
    {
      evalSettings.pureEval = false;
    }
  args.command->second->run();
}


/* -------------------------------------------------------------------------- */

}  /* End namespace `nix' */


/* ========================================================================== */

namespace flox {

/* -------------------------------------------------------------------------- */

FloxArgs::FloxArgs()
  : MultiCommand( nix::RegisterCommand::getCommandsFor( {} ) )
  , MixCommonArgs( "flox" )
{
  this->categories.clear();

  this->addFlag( {
    .longName    = "help"
  , .description = "Show usage information."
  , .category    = nix::miscCategory
  , .handler     = { [this]() { this->helpRequested = true; } }
  } );

  this->addFlag( {
    .longName    = "version"
  , .description = "Show version information."
  , .category    = nix::miscCategory
  , .handler     = { [this]() { this->showVersion = true; } }
  } );

  /* Added by `MixCommonArgs' */
  this->removeFlag( "option" );
  this->removeFlag( "log-format" );
  this->removeFlag( "max-jobs" );

  /* `MixCommonArgs' creates a flag for every config setting.
   * In practice we don't actually want users to set those, so we remove
   * them here. */
  std::map<std::string, nix::AbstractConfig::SettingInfo> settings;
  nix::globalConfig.getSettings( settings );
  for ( auto & [name, info] : settings )
    {
      if ( this->longFlags.find( name ) != this->longFlags.end() )
        {
          this->removeFlag( name );
        }
      if ( this->longFlags.find( "no-" + name ) != this->longFlags.end() )
        {
          this->removeFlag( "no-" + name );
        }
      if ( this->longFlags.find( "extra-" + name ) != this->longFlags.end() )
        {
          this->removeFlag( "extra-" + name );
        }
    }

  /* A special case setting flag added by `MixCommonArgs' that isn't handled
   * by the loop above. */
  this->removeFlag( "relaxed-sandbox" );
}


/* -------------------------------------------------------------------------- */


  nix::Strings::iterator
FloxArgs::rewriteArgs( nix::Strings           & args
                     , nix::Strings::iterator   pos
                     )
{
  if ( this->aliasUsed || this->command || ( pos == args.end() ) )
    {
      return pos;
    }
  auto arg = * pos;
  auto i   = this->aliases.find( arg );
  if ( i == this->aliases.end() ) { return pos; }
  nix::warn( "'%s' is a deprecated alias for '%s'"
           , arg
           , nix::concatStringsSep( " ", i->second )
           );
  pos = args.erase( pos );
  for ( auto j = i->second.rbegin(); j != i->second.rend(); ++j )
    {
      pos = args.insert( pos, * j );
    }
  this->aliasUsed = true;
  return pos;
}


/* -------------------------------------------------------------------------- */

  nlohmann::json
FloxArgs::dumpCli()
{
  auto res = nlohmann::json::object();

  res["args"] = this->toJSON();

  auto stores = nlohmann::json::object();
  for ( auto & implem : * nix::Implementations::registered )
    {
      auto storeConfig = implem.getConfig();
      auto storeName   = storeConfig->name();
      auto & j         = stores[storeName];
      j["doc"]         = storeConfig->doc();
      j["settings"]    = storeConfig->toJSON();
    }
  res["stores"] = std::move( stores );

  return res;
}


/* -------------------------------------------------------------------------- */

}  /* End namespace `flox' */


/* -------------------------------------------------------------------------- */

  int
main( int argc, char * argv[] )
{
  /* Increase the default stack size for the evaluator and for
   * libstdc++'s `std::regex'. */
  nix::setStackSize( 64 * 1024 * 1024 );

  return nix::handleExceptions( argv[0], [&]() {
      nix::mainWrapped( argc, argv );
  } );
}


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
