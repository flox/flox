/* ========================================================================== *
 *
 * @file flox/core/command.hh
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <memory>
#include <string>
#include <string_view>

#include <argparse/argparse.hpp>
#include <nix/flake/flakeref.hh>

#include "flox/core/exceptions.hh"
#include "flox/core/nix-state.hh"
#include "flox/core/types.hh"
#include "flox/core/util.hh"


/* -------------------------------------------------------------------------- */

/** @brief Executable command helpers, argument parsers, etc. */
namespace flox::command {

/* -------------------------------------------------------------------------- */

/**
 * @brief Add verbosity flags to any parser and modify the global verbosity.
 *
 * Nix verbosity levels for reference ( we have no `--debug` flag ):
 *   typedef enum {
 *     lvlError = 0   ( --quiet --quiet --quiet )
 *   , lvlWarn        ( --quiet --quiet )
 *   , lvlNotice      ( --quiet )
 *   , lvlInfo        ( **Default** )
 *   , lvlTalkative   ( -v )
 *   , lvlChatty      ( -vv   | --debug --quiet )
 *   , lvlDebug       ( -vvv  | --debug )
 *   , lvlVomit       ( -vvvv | --debug -v )
 *   } Verbosity;
 */
struct VerboseParser : public argparse::ArgumentParser
{
  explicit VerboseParser( const std::string & name,
                          const std::string & version = "0.1.0" );
}; /* End struct `VerboseParser' */

/**
 * @class flox::command::InvalidArgException
 * @brief An exception thrown when a command line argument is invalid.
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( InvalidArgException, EC_INVALID_ARG, "invalid argument" )
/** @} */


/* -------------------------------------------------------------------------- */

}  // namespace flox::command


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
