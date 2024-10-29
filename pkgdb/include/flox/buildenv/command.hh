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
#include <nlohmann/json.hpp>

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
  nlohmann::json             lockfileContent;
  std::optional<std::string> outLink;
  std::optional<System>      system;
  std::optional<std::string> storePath;
  std::optional<std::string> serviceConfigPath;
  bool                       buildContainer = false;
  std::optional<std::string> containerName;
  std::optional<std::string> containerTag;


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
