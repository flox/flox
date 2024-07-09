/* ========================================================================== *
 *
 * @file flox/linkenv/command.hh
 *
 * @brief Link a previously built environment.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <optional>
#include <string>

#include <nix/ref.hh>
#include <nlohmann/json.hpp>

#include "flox/core/command.hh"
#include "flox/core/nix-state.hh"
#include "flox/core/types.hh"
#include "flox/core/util.hh"


/* -------------------------------------------------------------------------- */

namespace flox::linkenv {

/* -------------------------------------------------------------------------- */

/** @brief Evaluate and build a locked environment. */
class LinkEnvCommand : NixState
{

private:

  command::VerboseParser     parser;
  std::optional<std::string> outLink;
  std::optional<std::string> storePath;


public:

  LinkEnvCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `linkenv` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End struct `LinkEnvCommand' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::linkenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
