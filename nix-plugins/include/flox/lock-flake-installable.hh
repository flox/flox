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

#include <nix/globals.hh>

/* -------------------------------------------------------------------------- */

namespace flox {

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
  /** The system the package reports in <drv>.system */
  std::string packageSystem;
  /** The system passed to pkgdb when locking an installable, which is used to
   * choose a default attribute path. */
  std::string                             system;
  std::string                             name;
  std::optional<std::string>              pname;
  std::optional<std::string>              version;
  std::optional<std::string>              description;
  std::optional<std::vector<std::string>> licenses;
  std::optional<bool>                     broken;
  std::optional<bool>                     unfree;
  std::optional<uint64_t>                 priority;

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
           && packageSystem == other.packageSystem && system == other.system
           && name == other.name && pname == other.pname
           && version == other.version && description == other.description
           && licenses == other.licenses && broken == other.broken
           && unfree == other.unfree;
  }

  [[nodiscard]] bool
  operator!=( const LockedInstallable & other ) const
  {
    return ! ( ( *this ) == other );
  }
};


void
to_json( nlohmann::json & jto, const LockedInstallable & from );

void
from_json( const nlohmann::json & jfrom, LockedInstallable & from );

/**
 * @brief Lock a flake installable, and evaluate critical metadata.
 * @param state The nix evaluation state
 * @param installableStr The flake installable to lock
 * @param system The system to lock the flake installable for. Used to determine
 * the package system if not specified by the installable
 */
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
