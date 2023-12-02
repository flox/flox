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
#include "flox/registry.hh"


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


/* -------------------------------------------------------------------------- */

/** @brief Extend a command's state blob with a single @a RegistryInput. */
class InlineInputMixin : virtual public NixState
{

private:

  RegistryInput registryInput;

protected:

  /**
   * @brief Fill @a registryInput by parsing a flake ref.
   * @param flakeRef A flake reference as a URL string or JSON attribute set.
   */
  void
  parseFlakeRef( const std::string & flakeRef )
  {
    this->registryInput.from
      = std::make_shared<nix::FlakeRef>( flox::parseFlakeRef( flakeRef ) );
  }


public:

  argparse::Argument &
  addSubtreeArg( argparse::ArgumentParser & parser );
  argparse::Argument &
  addFlakeRefArg( argparse::ArgumentParser & parser );

  /**
   * @brief Return the parsed @a RegistryInput.
   * @return The parsed @a RegistryInput.
   */
  [[nodiscard]] const RegistryInput &
  getRegistryInput()
  {
    return this->registryInput;
  }

}; /* End struct `InlineInputMixin' */


/* -------------------------------------------------------------------------- */

/** @brief Extend a command state blob with an attribute path to "target". */
struct AttrPathMixin
{

  flox::AttrPath attrPath;

  /**
   * @brief Sets the attribute path to be scraped.
   *
   * If no system is given use the current system.
   */
  argparse::Argument &
  addAttrPathArgs( argparse::ArgumentParser & parser );

  /**
   * @brief Sets fallback `attrPath` to a package set.
   *
   * If `attrPath` is empty use, `packages.<SYTEM>`.
   * If `attrPath` is one element then add "current system" as `<SYSTEM>`.
   */
  void
  fixupAttrPath();

}; /* End struct `AttrPathMixin' */


/* -------------------------------------------------------------------------- */

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
