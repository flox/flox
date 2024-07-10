/* ========================================================================== *
 *
 * @file flox/lock-flake-installable.hh
 *
 * @brief Executable command helper and `flox::lockFlakeInstallable`.
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

/** @brief Lock a flake installable for flox */
class LockFlakeInstallableCommand : flox::NixState
{

private:

  command::VerboseParser parser;

  std::string installable;
  std::string system = nix::settings.thisSystem.get();

public:

  LockFlakeInstallableCommand();

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
  std::string                          lockedUrl;
  std::optional<std::string>           flakeDescription;
  std::string                          lockedFlakeAttrPath;
  std::string                          derivation;
  std::map<std::string, std::string>   outputs;
  std::vector<std::string>             outputNames;
  std::optional<std::set<std::string>> outputsToInstall;
  std::optional<std::set<std::string>> requestedOutputsToInstall;
  std::string                          packageSystem;
  std::string                          lockedSystem;
  std::string                          name;
  std::optional<std::string>           pname;
  std::optional<std::string>           version;
  std::optional<std::string>           description;
  std::optional<std::string>           license;
  std::optional<bool>                  broken;
  std::optional<bool>                  unfree;

  [[nodiscard]] bool
  operator==( const LockedInstallable & other ) const
  {
    return lockedUrl == other.lockedUrl
           && flakeDescription == other.flakeDescription
           && lockedFlakeAttrPath == other.lockedFlakeAttrPath
           && derivation == other.derivation && outputs == other.outputs
           && outputNames == other.outputNames
           && outputsToInstall == other.outputsToInstall
           && requestedOutputsToInstall == other.requestedOutputsToInstall
           && packageSystem == other.packageSystem
           && lockedSystem == other.lockedSystem && name == other.name
           && pname == other.pname && version == other.version
           && description == other.description && license == other.license
           && broken == other.broken && unfree == other.unfree;
  }

  [[nodiscard]] bool
  operator!=( const LockedInstallable & other ) const
  {
    return ! ( ( *this ) == other );
  }
};


void
to_json( nlohmann::json & jto, const LockedInstallable & from );

/**
 * @brief Lock a flake installable, and evaluate critical metadata.
 * @param state The nix evaluation state
 * @param system The system to lock the flake installable for. Used to determine
 * the package system if not specified by the installable
 * @param installableStr The flake installable to lock
 */
LockedInstallable
lockFlakeInstallable( const nix::ref<nix::EvalState> & state,
                      const std::string &              installableStr,
                      const std::string &              system );


/**
 * @class flox::LockFlakeInstallableException
 * @brief An exception thrown when locking a flake installble to a
 * @a flox::LockedInstallable.
 *
 * @{
 */
FLOX_DEFINE_EXCEPTION( LockFlakeInstallableException,
                       EC_NIX_LOCK_FLAKE,
                       "Failed to lock flake installable" )
/** @} */

}  // namespace flox


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
