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

struct CmdBuildEnv : nix::EvalCommand
{
  std::string                 lockfileContent;
  std::optional<nix::Path>    outLink;
  std::optional<System>       system;

  CmdBuildEnv();

  void
  run( ref<nix::Store> store ) override;


};  /* End struct `CmdBuildEnv' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::buildenv


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
