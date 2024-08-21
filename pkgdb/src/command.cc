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

}  // namespace flox::command


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
