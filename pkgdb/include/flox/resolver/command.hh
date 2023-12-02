/* ========================================================================== *
 *
 * @file flox/resolver/command.hh
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <optional>

#include "flox/resolver/manifest-raw.hh"
#include "flox/resolver/mixins.hh"
#include "flox/search/command.hh"


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

/** @brief Lock a manifest file. */
class LockCommand : public GAEnvironmentMixin
{

private:

  command::VerboseParser parser;


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


}; /* End class `LockCommand' */


/* -------------------------------------------------------------------------- */

/** @brief Diff two manifest files. */
class DiffCommand
{

private:

  std::optional<std::filesystem::path> manifestPath;
  std::optional<ManifestRaw>           manifestRaw;

  std::optional<std::filesystem::path> oldManifestPath;
  std::optional<ManifestRaw>           oldManifestRaw;

  command::VerboseParser parser;


  [[nodiscard]] const ManifestRaw &
  getManifestRaw();

  [[nodiscard]] const ManifestRaw &
  getOldManifestRaw();


public:

  DiffCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `diff` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `DiffCommand' */


/* -------------------------------------------------------------------------- */

/** @brief Update lockfile inputs. */
class UpdateCommand : public GAEnvironmentMixin
{

private:

  std::optional<std::string> inputName;

  command::VerboseParser parser;


public:

  UpdateCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `update` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `UpdateCommand' */


/* -------------------------------------------------------------------------- */

/** @brief Show information about an environment's registries. */
class RegistryCommand : public GAEnvironmentMixin
{

private:

  command::VerboseParser parser;


public:

  RegistryCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `registry` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `DiffCommand' */


/* -------------------------------------------------------------------------- */

class ManifestCommand
{

private:

  command::VerboseParser parser;      /**< `manifest`          parser */
  LockCommand            cmdLock;     /**< `manifest lock`     command */
  DiffCommand            cmdDiff;     /**< `manifest diff`     command */
  UpdateCommand          cmdUpdate;   /**< `manifest update`   command */
  RegistryCommand        cmdRegistry; /**< `manifest registry` command */


public:

  ManifestCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `manifest` sub-command.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `ManifestCommand' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
