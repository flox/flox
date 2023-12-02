/* ========================================================================== *
 *
 * @file flox/pkgdb/command.hh
 *
 * @brief Executable command helpers, argument parsers, etc.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include "flox/core/command.hh"
#include "flox/pkgdb/input.hh"
#include "flox/pkgdb/write.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/** @brief Adds a single package database path to a state blob. */
struct DbPathMixin
{

  std::optional<std::filesystem::path> dbPath;

  /** Extend an argument parser to accept a `-d,--database PATH` argument. */
  argparse::Argument &
  addDatabasePathOption( argparse::ArgumentParser & parser );


}; /* End struct `DbPathMixin' */


/* -------------------------------------------------------------------------- */

/**
 * @brief Adds a single package database and optionally an associated flake to a
 *        state blob.
 */
template<pkgdb_typename T>
struct PkgDbMixin
  : virtual public DbPathMixin
  , virtual public command::InlineInputMixin
{

  std::shared_ptr<FloxFlake> flake;
  std::shared_ptr<T>         db;

  /**
   * @brief Open a @a flox::pkgdb::PkgDb connection using the command state's
   *        @a dbPath or @a flake value.
   */
  void
  openPkgDb();

  /**
   * @brief Add `target` argument to any parser to read either a `flake-ref` or
   *        path to an existing database.
   */
  argparse::Argument &
  addTargetArg( argparse::ArgumentParser & parser );


}; /* End struct `PkgDbMixin' */


/* -------------------------------------------------------------------------- */

/**
 * @brief Scrape a flake prefix producing a SQLite3 database with
 *        package metadata.
 */
class ScrapeCommand
  : public DbPathMixin
  , public command::AttrPathMixin
  , public command::InlineInputMixin
{

private:

  command::VerboseParser    parser;
  std::optional<PkgDbInput> input;
  /** Whether to force re-evaluation. */
  bool force = false;

  /** @brief Initialize @a input from @a registryInput. */
  void
  initInput();


public:

  ScrapeCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `scrape` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `ScrapeCommand' */


/* -------------------------------------------------------------------------- */

/**
 * @brief Minimal set of DB queries, largely focused on looking up info that is
 *        non-trivial to query with a "plain" SQLite statement.
 *
 * This subcommand has additional subcommands:
 * - `pkgdb get id [--pkg] DB-PATH ATTR-PATH...`
 *   + Lookup `(AttrSet|Packages).id` for `ATTR-PATH`.
 * - `pkgdb get done DB-PATH ATTR-PATH...`
 *   + Lookup whether `AttrPath` has been scraped.
 * - `pkgdb get path [--pkg] DB-PATH ID`
 *   + Lookup `AttrPath` for `(AttrSet|Packages).id`.
 * - `pkgdb get flake DB-PATH`
 *   + Dump the `LockedFlake` table including fingerprint, locked-ref, etc.
 * - `pkgdb get db FLAKE-REF`
 *   + Print the absolute path to the associated flake's db.
 */
class GetCommand
  : public PkgDbMixin<PkgDbReadOnly>
  , public command::AttrPathMixin
{

private:

  command::VerboseParser parser; /**< `get`       parser */
  command::VerboseParser pId;    /**< `get id`    parser */
  command::VerboseParser pPath;  /**< `get path`  parser */
  command::VerboseParser pDone;  /**< `get done`  parser */
  command::VerboseParser pFlake; /**< `get flake` parser */
  command::VerboseParser pDb;    /**< `get db`    parser */
  command::VerboseParser pPkg;   /**< `get pkg`   parser */
  bool                   isPkg = false;
  row_id                 id    = 0;

  /**
   * @brief Execute the `get id` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  runId();

  /**
   * @brief Execute the `get done` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  runDone();

  /**
   * @brief Execute the `get path` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  runPath();

  /**
   * @brief Execute the `get flake` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  runFlake();

  /**
   * @brief Execute the `get db` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  runDb();

  /**
   * @brief Execute the `get pkg` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  runPkg();


public:

  GetCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `get` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `GetCommand' */


/* -------------------------------------------------------------------------- */

class ListCommand
{

private:

  command::VerboseParser               parser;
  std::optional<std::filesystem::path> cacheDir;
  bool                                 json      = false;
  bool                                 basenames = false;


public:

  ListCommand();

  [[nodiscard]] command::VerboseParser &
  getParser()
  {
    return this->parser;
  }

  /**
   * @brief Execute the `list` routine.
   * @return `EXIT_SUCCESS` or `EXIT_FAILURE`.
   */
  int
  run();


}; /* End class `ListCommand' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
