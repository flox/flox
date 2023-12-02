/* ========================================================================== *
 *
 * @file flox/pkgdb/db-package.hh
 *
 * @brief Package metadata loaded from a `PkgDb' cache.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

#include <filesystem>
#include <string>

#include <nix/flake/flakeref.hh>

#include "flox/core/types.hh"
#include "flox/pkgdb/pkg-query.hh"
#include "flox/pkgdb/read.hh"
#include "flox/raw-package.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/** @brief Package metadata loaded from a `PkgDb' cache. */
class DbPackage : public RawPackage
{

protected:

  /* From `RawPackage':
   *   AttrPath                    path;
   *   std::string                 name;
   *   std::string                 pname;
   *   std::optional<std::string>  version;
   *   std::optional<std::string>  semver;
   *   std::optional<std::string>  license;
   *   std::vector<std::string>    outputs;
   *   std::vector<std::string>    outputsToInstall;
   *   std::optional<bool>         broken;
   *   std::optional<bool>         unfree;
   *   std::optional<std::string>  description;
   */

  row_id                pkgId;  /**< `Packages.id' in the database. */
  std::filesystem::path dbPath; /**< Path to the database. */

private:

  /** @brief Fill @a flox::RawPackage fields by reading them from @a pkgdb. */
  void
  initRawPackage( PkgDbReadOnly & pkgdb );


public:

  DbPackage( PkgDbReadOnly & pkgdb, row_id pkgId )
    : pkgId( pkgId ), dbPath( pkgdb.dbPath )
  {
    this->path = pkgdb.getPackagePath( pkgId );
    this->initRawPackage( pkgdb );
  }

  DbPackage( PkgDbReadOnly & pkgdb, const AttrPath & path )
    : pkgId( pkgdb.getPackageId( path ) ), dbPath( pkgdb.dbPath )
  {
    this->path = path;
    this->initRawPackage( pkgdb );
  }

  /** @return The `Packages.id` of the package. */
  row_id
  getPackageId() const
  {
    return this->pkgId;
  }

  /** @return The path to the database. */
  std::filesystem::path
  getDbPath() const
  {
    return this->dbPath;
  }

  /** @return The locked _flake reference_ where the package is defined. */
  nix::FlakeRef
  getLockedFlakeRef() const
  {
    return PkgDbReadOnly( this->dbPath.string() ).getLockedFlakeRef();
  }

}; /* End class `DbPackage' */


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
