/* ========================================================================== *
 *
 * @file flox/pkgdb/input.hh
 *
 * @brief A @a RegistryInput that opens a @a PkgDb associated with a flake.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <memory>
#include <nlohmann/json_fwd.hpp>
#include <optional>
#include <string>
#include <string_view>
#include <type_traits>
#include <utility>
#include <vector>

#include <nix/flake/flake.hh>
#include <nix/flake/flakeref.hh>
#include <nix/ref.hh>

#include "flox/core/nix-state.hh"
#include "flox/core/types.hh"
#include "flox/flox-flake.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/pkgdb/read.hh"
#include "flox/registry.hh"


/* -------------------------------------------------------------------------- */

/* Forward declare */
namespace nix {
class Store;
}


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/* Forward declare */
class PkgDb;

/* -------------------------------------------------------------------------- */

/** @brief A @a RegistryInput that opens a @a PkgDb associated with a flake. */
class PkgDbInput : public FloxFlakeInput
{

private:

  /* Exit code used during multi-process scraping to indicate successful
   * processing but additinal pages of attributes are yet to be processed. */
  static const int EXIT_CHILD_INCOMPLETE = EXIT_SUCCESS + 1;
  /* Exit code used during multi-process scraping to indicate an unrecoverable
   * error occured in the nix evalutaion. Chosen arbitrarily, but with the
   * intent to avoid posix overlap. */
  static const int EXIT_FAILURE_NIX_EVAL = 150;

  /* Provided by `FloxFlakeInput':
   *   nix::ref<nix::FlakeRef>             flakeRef
   *   nix::ref<nix::Store>                store
   *   std::shared_ptr<FloxFlake>          flake
   *   std::optional<std::vector<Subtree>> enabledSubtrees
   */

  /** Path to the flake's pkgdb SQLite3 file. */
  std::filesystem::path dbPath;

  /**
   * A read-only database connection that remains open for the lifetime of
   * @a this object.
   */
  std::shared_ptr<PkgDbReadOnly> dbRO;

  /**
   * A read/write database connection that may be opened and closed as needed
   * using @a getDbReadWrite and @a closeDbReadWrite.
   */
  std::shared_ptr<PkgDb> dbRW;

  /** The name of the input, used to emit output with shortnames. */
  std::optional<std::string> name;

  /**
   * @brief Prepare database handles for use.
   *
   * Upon exiting a compatible read-only database connection will be open
   * with the `LockedFlake` and `DbVersions` tables created.
   *
   * If the database does not exist it will be created.
   *
   * If the database `VIEW`s schemas are out of date they will be updated.
   *
   * If the database `TABLE`s schemas are out of date the database will be
   * deleted and recreated.
   */
  void
  init();
  bool
  initDbRO();


public:

  /**
   * @brief Tag used to disambiguate construction with database path and
   *        cache directory path.
   */
  struct db_path_tag
  {};

  /** Heuristically determined limits for page size for scraping.  This affects
   * memory usage.  See @a getScrapingPageSize()
   */
  static const size_t maxPageSize = 100 * 1000;
  static const size_t minPageSize = 1 * 1000;

  /**
   * @brief Construct a @a PkgDbInput from a @a RegistryInput and a path to
   *        the database.
   * @param store A `nix` store connection.
   * @param input A @a RegistryInput.
   * @param dbPath Path to the database.
   * @param db_path_tag Tag used to disambiguate this constructor from
   *                    other constructor which takes a cache directory.
   * @param name Name of the input ( empty implies N/A ).
   */
  PkgDbInput( nix::ref<nix::Store> & store,
              const RegistryInput &  input,
              std::filesystem::path  dbPath,
              const db_path_tag & /* unused */
              ,
              const std::string & name = "" )
    : FloxFlakeInput( store, input )
    , dbPath( std::move( dbPath ) )
    , name( name.empty() ? std::nullopt : std::make_optional( name ) )
  {
    this->init();
  }

  /**
   * @brief Construct a @a PkgDbInput from a @a RegistryInput and a path to
   *        the directory where the database should be cached.
   * @param store A `nix` store connection.
   * @param input A @a RegistryInput.
   * @param cacheDir Path to the directory where the database should
   *                 be cached.
   * @param name Name of the input ( empty implies N/A ).
   */
  PkgDbInput( nix::ref<nix::Store> &        store,
              const RegistryInput &         input,
              const std::filesystem::path & cacheDir = getPkgDbCachedir(),
              const std::string &           name     = "" )
    : FloxFlakeInput( store, input )
    , dbPath( genPkgDbName( this->getFlake()->lockedFlake.getFingerprint(),
                            cacheDir ) )
    , name( name.empty() ? std::nullopt : std::make_optional( name ) )
  {
    this->init();
  }

  /**
   * @return The read-only database connection handle.
   */
  [[nodiscard]] nix::ref<PkgDbReadOnly>
  getDbReadOnly() const
  {
    return static_cast<nix::ref<PkgDbReadOnly>>( this->dbRO );
  }

  /**
   * @brief Open a read/write database connection if one is not open, and
   *        return a handle.
   */
  [[nodiscard]] nix::ref<PkgDb>
  getDbReadWrite();

  /** @brief Close the read/write database connection if it is open. */
  void
  closeDbReadWrite();

  /** @return Filesystem path to the flake's package database. */
  [[nodiscard]] std::filesystem::path
  getDbPath() const
  {
    return this->dbPath;
  }

  /**
   * @brief Scrape all prefixes indicated by @a InputPreferences for
   *        @a systems.
   * @param systems Systems to be scraped.
   */
  void
  scrapeSystems( const std::vector<System> & systems );

  /**
   * @brief Ensure that an attribute path prefix has been scraped.
   *
   * If the prefix has been scraped no writes are performed, but if the prefix
   * has not been scraped a read/write connection will be used.
   *
   * If a read/write connection is already open when @a scrapePrefix is called
   * it will remain open, but if the connection is opened by @a scrapePrefix
   * it will be closed after scraping is completed.
   * @param prefix Attribute path to scrape.
   */
  void
  scrapePrefix( const flox::AttrPath & prefix );

  /**
   * @brief Scrapes one page of attributes directly beneath @a prefix.  Used
   * specifically as a child process in @a scrapePrefix. Attributes N to N + @a
   * pageSize where N is @a pageSize * @a pageIdx will be scraped, depth first.
   *
   * @param input The PkgDbInput to scrape from.  This is passed to this static
   * helper rather than relying on a method and using *this* to encourage
   * encapsulation.
   * @param prefix The prefix to process attributes beneath.
   * @param pageIdx The page of attributes to process
   * @param pageSize The number of attributes per page.
   */
  static int
  scrapePrefixWorker( PkgDbInput *     input,
                      const AttrPath & prefix,
                      const size_t     pageIdx,
                      const size_t     pageSize );

  /** @brief Add/set a shortname for this input. */
  void
  setName( std::string_view name )
  {
    this->name = name;
  }

  /**
   * @brief Get an identifier for this input.
   * @return The shortname of this input, or its locked flake-ref.
   */
  [[nodiscard]] std::string
  getNameOrURL()
  {
    return this->name.value_or(
      this->getFlake()->lockedFlake.flake.lockedRef.to_string() );
  }

  /** @brief Get a JSON representation of a row in the database. */
  [[nodiscard]] nlohmann::json
  getRowJSON( row_id row );

  /** @brief Return the name if it was provided. */
  [[nodiscard]] std::optional<std::string>
  getName() const
  {
    return this->name;
  }

  /**
   * @brief Helper to identify the pageSize to use for scraping.
   * @return pageSize in items
   */
  static int
  getScrapingPageSize();

}; /* End struct `PkgDbInput' */


/* -------------------------------------------------------------------------- */

/** @brief Factory for @a PkgDbInput. */
class PkgDbInputFactory
{

private:

  nix::ref<nix::Store>  store;    /**< `nix` store connection. */
  std::filesystem::path cacheDir; /**< Cache directory. */


public:

  using input_type = PkgDbInput;

  /** @brief Construct a factory using a `nix` evaluator. */
  explicit PkgDbInputFactory( nix::ref<nix::Store> & store,
                              std::filesystem::path  cacheDir
                              = getPkgDbCachedir() )
    : store( store ), cacheDir( std::move( cacheDir ) )
  {}

  /** @brief Construct an input from a @a RegistryInput. */
  [[nodiscard]] std::shared_ptr<PkgDbInput>
  mkInput( const std::string & name, const RegistryInput & input )
  {
    return std::make_shared<PkgDbInput>( this->store,
                                         input,
                                         this->cacheDir,
                                         name );
  }


}; /* End class `PkgDbInputFactory' */


static_assert( registry_input_factory<PkgDbInputFactory> );


/* -------------------------------------------------------------------------- */

/**
 * @brief Provides a registry of @a PkgDb managers.
 *
 * Derived classes must provide their own @a getRegistryRaw and @a getSystems
 * implementations to support @a initRegistry and @a scrapeIfNeeded.
 */
class PkgDbRegistryMixin : virtual protected NixStoreMixin
{

private:

  /* From `NixStoreMixin':
   *   std::shared_ptr<nix::Store> store
   */

  std::shared_ptr<Registry<PkgDbInputFactory>> registry;

  // TODO: Implement
  /** Whether to force re-evaluation of flakes. */
  bool force = false;


protected:

  /* From `NixStoreMixin':
   *   nix::ref<nix::Store> getStore()
   */


  /** @brief Initialize @a registry member from @a params.registry. */
  void
  initRegistry();

  /**
   * @brief Lazily perform scraping on input flakes.
   *
   * If scraping is necessary temprorary read/write handles are opened for
   * those flakes and closed before returning from this function.
   */
  void
  scrapeIfNeeded();

  /** @return A raw registry used to initialize. */
  [[nodiscard]] virtual RegistryRaw
  getRegistryRaw()
    = 0;

  /** @return A list of systems to be scraped. */
  [[nodiscard]] virtual const std::vector<System> &
  getSystems()
    = 0;


public:

  /**
   * @brief Get the set of package databases to resolve in.
   *
   * This lazily initializes the registry and scrapes inputs when necessary.
   */
  [[nodiscard]] nix::ref<Registry<PkgDbInputFactory>>
  getPkgDbRegistry();

  /** @brief Whether DBs will be regenerated from scratch. */
  [[nodiscard]] bool
  isPkgDbForced() const
  {
    return this->force;
  }

  /** @brief Set whether DBs will be regenerated from scratch. */
  void
  setPkgDbForced( bool force )
  {
    this->force = force;
  }


}; /* End class `PkgDbRegistryMixin' */


/* -------------------------------------------------------------------------- */


}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
