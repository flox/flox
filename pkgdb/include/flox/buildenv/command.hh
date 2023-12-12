/* ========================================================================== *
 *
 * @file flox/buildenv/command.hh
 *
 * @brief Evaluate and build a locked environment.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <optional>
#include <string>

#include <nix/ref.hh>

#include "flox/core/command.hh"
#include "flox/core/nix-state.hh"
#include "flox/core/types.hh"
#include "flox/core/util.hh"


/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

/** @brief Evaluate and build a locked environment. */
class BuildEnvCommand : NixState
{

private:

  command::VerboseParser     parser;
  std::string                lockfileContent;
  std::optional<std::string> outLink;
  std::optional<System>      system;


public:

  BuildEnvCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `buildenv` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End struct `BuildEnvCommand' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
