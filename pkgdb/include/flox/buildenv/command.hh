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

/**
 * @class flox::buildenv::SystemNotSupportedByLockfile
 * @brief An exception thrown when a lockfile is is missing a package.<system>
 * entry fro the requested system.
 * @{
 */
FLOX_DEFINE_EXCEPTION( BuildenvInvalidArguments,
                       EC_BUILDENV_ARGUMENTS,
                       "invalid arguments to buildenv" )
/** @} */

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
  bool                       buildContainer;


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
