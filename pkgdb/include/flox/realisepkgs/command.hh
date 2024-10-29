/* ========================================================================== *
 *
 * @file flox/realisepkgs/command.hh
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

namespace flox::realisepkgs {

/* -------------------------------------------------------------------------- */

/** @brief Evaluate and build a locked environment. */
class RealisePkgsCommand : NixState
{

private:

  command::VerboseParser     parser;
  nlohmann::json             lockfileContent;
  std::optional<System>      system;
  std::optional<std::string> storePath;


public:

  RealisePkgsCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `realisepkgs` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End struct `RealisePkgsCommand' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::realisepkgs


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
