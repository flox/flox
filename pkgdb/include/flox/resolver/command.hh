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

  std::optional<std::vector<std::string>> inputNames;

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

/** @brief Upgrade groups or standalone packages in an environment. */
class UpgradeCommand : public GAEnvironmentMixin
{

private:

  std::optional<std::vector<std::string>> groupsOrIIDS;

  command::VerboseParser parser;


public:

  UpgradeCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `upgrade` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `UpgradeCommand' */


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

/** @brief Check a locked manifest. */
class CheckCommand
{

private:

  command::VerboseParser parser;

  /** Raw contents of project's lockfile ( if any ). */
  std::optional<LockfileRaw> lockfileRaw;

  /** The project's lockfile ( if any ). */
  std::optional<Lockfile> lockfile;

  /** The system to check the lockfile for. */
  std::optional<flox::System> system;

protected:

  /**
   * @brief Set the @a lockfilePath member variable by loading a lockfile from
   * `path`.
   *
   * @throws @a EnvironmentMixinException if called after @a lockfile is
   * initialized, as it is no longer allowed to change the lockfile.
   * @throws @a InvalidLockfileException if the lockfile at `path` is invalid.
   */
  void
  setLockfileRaw( const std::filesystem::path & path );


public:


  /**
   * @brief Lazily initialize and return the @a lockfile.
   *
   * If @a lockfile is set simply return it.
   * If @a lockfile is unset, try to initialize it
   */
  [[nodiscard]] virtual Lockfile
  getLockfile();

  CheckCommand();

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

}; /* End class `CheckCommand' */


/* -------------------------------------------------------------------------- */

class ManifestCommand
{

private:

  command::VerboseParser parser;      /**< `manifest`          parser */
  LockCommand            cmdLock;     /**< `manifest lock`     command */
  DiffCommand            cmdDiff;     /**< `manifest diff`     command */
  UpdateCommand          cmdUpdate;   /**< `manifest update`   command */
  UpgradeCommand         cmdUpgrade;  /**< `manifest upgrade`  command */
  RegistryCommand        cmdRegistry; /**< `manifest registry` command */
  CheckCommand           cmdCheck;    /**< `manifest check`    command */


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
