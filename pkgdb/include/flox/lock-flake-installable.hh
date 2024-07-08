/* ========================================================================== *
 *
 * @file flox/eval.hh
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>

#include <argparse/argparse.hpp>

#include "flox/core/command.hh"
#include "flox/core/nix-state.hh"


/* -------------------------------------------------------------------------- */

namespace flox {

/* -------------------------------------------------------------------------- */

/** @brief Lock a falke installble for flox */
class LockCommand : flox::NixState
{

private:

  command::VerboseParser parser;

  std::string installable;
  std::string system = nix::settings.thisSystem.get();

public:

  LockCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `lock` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();
};


/* -------------------------------------------------------------------------- */

struct LockedInstallable
{
  std::string                        lockedUrl;
  std::optional<std::string>         flakeDescription;
  std::string                        lockedAttrPath;
  std::string                        derivation;
  std::map<std::string, std::string> outputs;
  std::set<std::string>              outputsToInstall;
  std::string                        system;
  std::string                        name;
  std::optional<std::string>         pname;
  std::optional<std::string>         version;
  std::optional<std::string>         description;
  std::optional<std::string>         license;
  std::optional<bool>                broken;
  std::optional<bool>                unfree;
};

void
to_json( nlohmann::json & jto, const LockedInstallable & from );

LockedInstallable
lockFlakeInstallable( const nix::ref<nix::EvalState> & state,
                      const std::string &              installableStr,
                      const std::string &              system );

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
