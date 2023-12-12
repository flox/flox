/* ========================================================================== *
 *
 * @file flox/buildenv/command.hh
 *
 * @brief Evaluate and build a locked environment.
 *
 *
 * -------------------------------------------------------------------------- */

#include "flox/command.hh"

/* -------------------------------------------------------------------------- */

namespace flox::buildenv {

/* -------------------------------------------------------------------------- */

/** @brief Evaluate and build a locked environment. */
class BuildEnvCommand
{
  {
    command::VerboseParser     parser;
    std::string                lockfileContent;
    std::optional<std::string> outLink;
    std::optional<System>      system;

    BuildEnvCommand();

    void run( ref<nix::Store> store ) override;


  }; /* End struct `BuildEnvCommand' */


  /* --------------------------------------------------------------------------
   */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
