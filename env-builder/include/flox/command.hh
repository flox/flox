/* ========================================================================== *
 *
 * @file flox/command.hh
 *
 * @brief Extensions to `libnixcmd` command line parsers.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <nix/command.hh>


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/*
 * Existing Nix categories from `nix/command.hh':
 *   static constexpr Command::Category catHelp = -1;
 *   static constexpr Command::Category catSecondary = 100;
 *   static constexpr Command::Category catUtility = 101;
 *   static constexpr Command::Category catNixInstallation = 102;
 *
 * Default is defined in `Command::catDefault':
 *  static constexpr Category catDefault = 0;
 */

/** Local Development Commands */
static constexpr nix::Command::Category catLocal = 201;

/** Sharing Commands */
static constexpr nix::Command::Category catSharing = 202;

/** Additional Commands */
static constexpr nix::Command::Category catAdditional = 203;


/* -------------------------------------------------------------------------- */

struct FloxArgs
  : virtual public nix::MultiCommand
  , virtual nix::MixCommonArgs
{
  bool useNet        = true;
  bool refresh       = false;
  bool helpRequested = false;
  bool showVersion   = false;

  FloxArgs();

  std::map<std::string, std::vector<std::string>> aliases = {
    //   { "dev-shell",     { "develop"                } }
    // , { "diff-closures", { "store", "diff-closures" } }
  };

  bool aliasUsed = false;

  nix::Strings::iterator
  rewriteArgs( nix::Strings & args, nix::Strings::iterator pos ) override;

  std::string
  description() override
  {
    return "a tool for reproducible and declarative environment management";
  }

  std::string
  doc() override
  {
    return "TODO";
  }

  /* Plugins may add new subcommands. */
  void
  pluginsInited() override
  {
    this->commands = nix::RegisterCommand::getCommandsFor( {} );
  }

  nlohmann::json
  dumpCli();

}; /* End struct `FloxArgs' */


/* -------------------------------------------------------------------------- */

void
showSubcommandHelp( FloxArgs & toplevel, nix::MultiCommand & command );

void
showHelp( std::vector<std::string> subcommand, FloxArgs & toplevel );


/* -------------------------------------------------------------------------- */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
