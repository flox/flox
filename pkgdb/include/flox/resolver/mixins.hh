/* ========================================================================== *
 *
 * @file flox/resolver/mixins.hh
 *
 * @brief State blobs for flox commands.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <optional>
#include <string_view>

#include "flox/core/exceptions.hh"
#include "flox/resolver/environment.hh"
#include "flox/resolver/lockfile.hh"
#include "flox/resolver/manifest.hh"


/* -------------------------------------------------------------------------- */

/* Forward Declarations */

namespace argparse {

class Argument;
class ArgumentParser;

}  // namespace argparse


/* -------------------------------------------------------------------------- */

namespace flox::resolver {

/* -------------------------------------------------------------------------- */

/**
 * @a class flox::resolver::EnvironmentMixinException
 * @brief An exception thrown by @a flox::resolver::EnvironmentMixin during
 *        its initialization.
 * @{
 */
FLOX_DEFINE_EXCEPTION( EnvironmentMixinException,
                       EC_ENVIRONMENT_MIXIN,
                       "error handling manifest or lockfile" )
/** @} */


/* -------------------------------------------------------------------------- */

/**
 * @brief A state blob with files associated with an environment.
 *
 * This structure stashes several fields to avoid repeatedly calculating them.
 */
class EnvironmentMixin
{

private:

  /* All member variables are calculated lazily using `std::optional' and
   * `get<MEMBER>' accessors.
   * Even for internal access you should use the `get<MEMBER>' accessors to
   * lazily initialize. */

  /* ------------------------------ arguments ------------------------------- */


  /** Path to project level manifest. ( required ) */
  std::optional<std::filesystem::path> manifestPath;

  /* ----------------------------- lazy fields ------------------------------ */


  /**
   * Contents of user level manifest with global registry and settings
   * ( if any ).
   */
  std::optional<GlobalManifest> globalManifest;

  /**
   * Contents of project level manifest with registry, settings,
   * activation hook, and list of packages.
   */
  std::optional<ManifestRaw> manifestRaw;

  /**
   * Contents of project level manifest with registry, settings,
   * activation hook, and list of packages. ( required )
   */
  std::optional<EnvironmentManifest> manifest;

  /**
   * Contents of project level manifest with registry, settings,
   * activation hook, and list of packages.
   */
  std::optional<GlobalManifestRaw> globalManifestRaw;

  /** Raw contents of project's lockfile ( if any ). */
  std::optional<LockfileRaw> lockfileRaw;

  /** Contents of project's lockfile ( if any ). */
  std::optional<Lockfile> lockfile;

  /** Lazily initialized environment wrapper. */
  std::optional<Environment> environment;


protected:

  /**
   * @brief Set @a globalManifestRaw by loading a manifest from `maybePath`.
   * Overrides any previous value before @a manifest is initialized.
   *
   *
   * @throws @a EnvironmentMixinException if called after @a globalManifest is
   * initialized, as it is no longer allowed to change the global manifest.
   * @throws @a EnvironmentMixinException the path does not exist.
   */
  void
  setGlobalManifestRaw( std::optional<std::filesystem::path> maybePath );

  /**
   * @brief Manually set @a globalManifestRaw.
   * Overrides any previous value before @a manifest is initialized.
   *
   *
   * @throws @a EnvironmentMixinException if called after @a globalManifest is
   * initialized, as it is no longer allowed to change the global manifest.
   * @throws @a EnvironmentMixinException the path does not exist.
   */
  void
  setGlobalManifestRaw( std::optional<GlobalManifestRaw> maybeRaw );


  /**
   * @brief Initialize the @a globalManifest member variable.
   *
   * This is called by @a getGlobalManifest() to lazily initialize the global
   * manifest.
   *
   * This function exists so that child classes can change how their global
   * manifest is initialized.
   */
  [[nodiscard]] virtual GlobalManifest
  initGlobalManifest( GlobalManifestRaw manifestRaw );

  /**
   * @brief Set @a manifestRaw by loading a manifest from @a `maybePath`.
   *
   * Overrides any previous value before @a manifest is initialized.
   *
   * @throws @a EnvironmentMixinException if called after @a manifest is
   * initialized, as it is no longer allowed to change the manifest.
   * @throws @a EnvironmentMixinException the path does not exist.
   */
  void
  setManifestRaw( std::optional<std::filesystem::path> maybePath );

  /**
   * @brief Manually set @a manifestRaw.
   *
   * Overrides any previous value before @a manifest is initialized.
   *
   * @throws @a EnvironmentMixinException if called after @a manifest is
   * initialized, as it is no longer allowed to change the manifest.
   * @throws @a EnvironmentMixinException the path does not exist.
   */
  void
  setManifestRaw( std::optional<ManifestRaw> maybeRaw );


  /**
   * @brief Initialize the @a manifest member variable.
   *
   * Creates a @a flox::resolver::EnvironmentManifest from @a manifestRaw
   * stored in the current instance.
   *
   * This function exists so that child classes can override how their manifest
   * is initialized.
   */
  [[nodiscard]] virtual EnvironmentManifest
  initManifest( ManifestRaw manifestRaw );

  /**
   * @brief Set the @a lockfilePath member variable by loading a lockfile from
   * `path`.
   *
   * @throws @a EnvironmentMixinException if called after @a lockfile is
   * initialized, as it is no longer allowed to change the lockfile.
   */
  virtual void
  setLockfileRaw( std::filesystem::path path );

  /**
   * @brief Set the @a lockfilePath member variable.
   *
   * @throws @a EnvironmentMixinException if called after @a lockfile is
   * initialized, as it is no longer allowed to change the lockfile.
   */
  virtual void
  setLockfileRaw( LockfileRaw lockfileRaw );

  /**
   * @brief Initialize a @a flox::resolver::Lockfile from @a lockfileRaw.
   *
   * If @a lockfilePath is not set return an empty @a std::optional.
   */
  [[nodiscard]] virtual Lockfile
  initLockfile( LockfileRaw lockfileRaw );


  [[nodiscard]] const std::optional<LockfileRaw> &
  getLockfileRaw()
  {
    return this->lockfileRaw;
  }


public:

  /** @brief Get raw global manifest ( if any ). */
  [[nodiscard]] const std::optional<GlobalManifestRaw> &
  getGlobalManifestRaw()
  {
    return this->globalManifestRaw;
  }


  /**
   * @brief Lazily initialize and return the @a globalManifest.
   *
   * If @a globalManifest is set simply return it.
   * If @a globalManifest is unset, try to initialize it using
   * @a initGlobalManifest().
   */
  [[nodiscard]] const std::optional<GlobalManifest>
  getGlobalManifest();

  /** @brief Get the filesystem path to the manifest ( if any ). */
  [[nodiscard]] const std::optional<ManifestRaw> &
  getManifestRaw() const
  {
    return this->manifestRaw;
  }

  /**
   * @brief Lazily initialize and return the @a manifest.
   *
   * If @a manifest is set simply return it.
   * If @a manifest is unset, initialize it using @a initManifest().
   */
  [[nodiscard]] const EnvironmentManifest &
  getManifest();

  /**
   * @brief Lazily initialize and return the @a lockfile.
   *
   * If @a lockfile is set simply return it.
   * If @a lockfile is unset, but @a lockfilePath is set then
   * load from the file.
   */
  [[nodiscard]] const std::optional<Lockfile> &
  getLockfile();

  /**
   * @brief Laziliy initialize and return the @a environment.
   *
   * Member variables associated with the _global manifest_ and _lockfile_
   * are optional.
   *
   * @throws @a EnvironmentMixinException if the @a getManifest() returns an
   * empty optional.
   */
  [[nodiscard]] Environment &
  getEnvironment();

  /* -------------------------- argument parsers ---------------------------- */

  /**
   * @brief Sets the path to the global manifest file to load
   *        with `--global-manifest`.
   * @param parser The parser to add the argument to.
   * @return The argument added to the parser.
   */
  argparse::Argument &
  addGlobalManifestFileOption( argparse::ArgumentParser & parser );

  /**
   * @brief Sets the path to the manifest file to load with `--manifest`.
   * @param parser The parser to add the argument to.
   * @param required Whether the argument is required.
   * @return The argument added to the parser.
   */
  argparse::Argument &
  addManifestFileOption( argparse::ArgumentParser & parser );

  /**
   * @brief Sets the path to the manifest file to load with a positional arg.
   * @param parser The parser to add the argument to.
   * @param required Whether the argument is required.
   * @return The argument added to the parser.
   */
  argparse::Argument &
  addManifestFileArg( argparse::ArgumentParser & parser, bool required = true );

  /**
   * @brief Sets the path to the old lockfile to load with `--lockfile`.
   * @param parser The parser to add the argument to.
   * @return The argument added to the parser.
   */
  argparse::Argument &
  addLockfileOption( argparse::ArgumentParser & parser );

  /**
   * @brief Uses a `--dir PATH` to locate `manifest.{toml,yaml,json}' file and
   *        `manifest.lock` if it is present.`.
   * @param parser The parser to add the argument to.
   * @return The argument added to the parser.
   */
  argparse::Argument &
  addFloxDirectoryOption( argparse::ArgumentParser & parser );


}; /* End class `EnvironmentMixin' */


/* -------------------------------------------------------------------------- */

class GAEnvironmentMixin : public EnvironmentMixin
{

private:

  /** Whether to override manifest registries for use with `flox` GA. */
  bool gaRegistry = false;


protected:

  /**
   * @brief Initialize the @a globalManifest member variable.
   *        When `--ga-registry` is set it enforces a GA compliant manifest by
   *        disallowing `registry` in its input,
   *        and injects a hard coded `registry`.
   */
  [[nodiscard]] GlobalManifest
  initGlobalManifest( GlobalManifestRaw manifestRaw ) override;

  /**
   * @brief Initialize the @a manifest member variable.
   *        When `--ga-registry` is set it enforces a GA compliant manifest by
   *        disallowing `registry` in its input,
   *        and injects a hard coded `registry`.
   */
  [[nodiscard]] EnvironmentManifest
  initManifest( ManifestRaw manifestRaw ) override;


public:

  /**
   * @brief Hard codes a manifest with only `github:NixOS/nixpkgs/release-23.05`
   * with `--ga-registry`.
   * @param parser The parser to add the argument to.
   * @return The argument added to the parser.
   */
  argparse::Argument &
  addGARegistryOption( argparse::ArgumentParser & parser );

}; /* End class `GAEnvironmentMixin' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::resolver


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
