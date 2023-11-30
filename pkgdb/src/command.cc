/* ========================================================================== *
 *
 * @file command.cc
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#include <algorithm>
#include <optional>
#include <string>
#include <variant>
#include <vector>

#include <argparse/argparse.hpp>
#include <nix/config.hh>
#include <nix/error.hh>
#include <nix/globals.hh>
#include <nix/logging.hh>

#include "flox/core/command.hh"
#include "flox/core/types.hh"
#include "flox/registry.hh"


/* -------------------------------------------------------------------------- */

namespace flox::command {

/* -------------------------------------------------------------------------- */

VerboseParser::VerboseParser( const std::string & name,
                              const std::string & version )
  : argparse::ArgumentParser( name, version, argparse::default_arguments::help )
{
  this->add_argument( "-q", "--quiet" )
    .help( "decrease the logging verbosity level. May be used up to 3 times." )
    .action(
      [&]( const auto & )
      {
        nix::verbosity = ( nix::verbosity <= nix::lvlError )
                           ? nix::lvlError
                           : static_cast<nix::Verbosity>( nix::verbosity - 1 );
      } )
    .default_value( false )
    .implicit_value( true )
    .append();

  this->add_argument( "-v", "--verbose" )
    .help( "increase the logging verbosity level. May be used up to 4 times." )
    .action(
      [&]( const auto & )
      {
        nix::verbosity = ( nix::lvlVomit <= nix::verbosity )
                           ? nix::lvlVomit
                           : static_cast<nix::Verbosity>( nix::verbosity + 1 );
      } )
    .default_value( false )
    .implicit_value( true )
    .append();
}


/* -------------------------------------------------------------------------- */

argparse::Argument &
InlineInputMixin::addFlakeRefArg( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "flake-ref" )
    .help( "flake-ref URI string or JSON attrs ( preferably locked )" )
    .required()
    .metavar( "FLAKE-REF" )
    .action( [&]( const std::string & flakeRef )
             { this->parseFlakeRef( flakeRef ); } );
}


argparse::Argument &
InlineInputMixin::addSubtreeArg( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "--subtree" )
    .help( "a subtree name, being one of `packages` or `legacyPackages`, "
           "that should be processed. May be used multiple times." )
    .required()
    .metavar( "SUBTREE" )
    .action(
      [&]( const std::string & subtree )
      {
        /* Parse the subtree type to an enum. */
        Subtree stype = Subtree::parseSubtree( subtree );
        /* Create or append the `subtrees' list. */
        if ( this->registryInput.subtrees.has_value() )
          {
            if ( auto has = std::find( this->registryInput.subtrees->begin(),
                                       this->registryInput.subtrees->end(),
                                       stype );
                 has == this->registryInput.subtrees->end() )
              {
                this->registryInput.subtrees->emplace_back( stype );
              }
          }
        else
          {
            this->registryInput.subtrees
              = std::make_optional( std::vector<Subtree> { stype } );
          }
      } );
}


/* -------------------------------------------------------------------------- */

argparse::Argument &
AttrPathMixin::addAttrPathArgs( argparse::ArgumentParser & parser )
{
  return parser.add_argument( "attr-path" )
    .help( "attribute path to scrape" )
    .metavar( "ATTRS..." )
    .remaining()
    .action( [&]( const std::string & path )
             { this->attrPath.emplace_back( path ); } );
}


void
AttrPathMixin::fixupAttrPath()
{
  if ( this->attrPath.empty() ) { this->attrPath.push_back( "packages" ); }
  if ( this->attrPath.size() < 2 )
    {
      this->attrPath.push_back( nix::settings.thisSystem.get() );
    }
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::command


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
