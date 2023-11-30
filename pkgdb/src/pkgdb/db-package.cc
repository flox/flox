/* ========================================================================== *
 *
 * @file pkgdb/db-package.cc
 *
 * @brief Package metadata loaded from a `PkgDb' cache.
 *
 *
 * -------------------------------------------------------------------------- */

#include "flox/pkgdb/db-package.hh"


/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

// TODO: Conversion by JSON isn't efficient. Read values directly.
void
DbPackage::initRawPackage( PkgDbReadOnly & pkgdb )
{
  sqlite3pp::query qry( pkgdb.db, R"SQL(
      SELECT json_object(
        'name',             name
      , 'pname',            pname
      , 'version',          version
      , 'semver',           semver
      , 'license',          license
      , 'outputs',          json( outputs )
      , 'outputsToInstall', json( outputsToInstall )
      , 'broken',           iif( broken, json( 'true' ), json( 'false' ) )
      , 'unfree',           iif( broken, json( 'true' ), json( 'false' ) )
      , 'description',      description
      ) AS json
      FROM Packages
      LEFT OUTER JOIN Descriptions
        ON ( Packages.descriptionId = Descriptions.id  )
      WHERE ( Packages.id = ? )
    )SQL" );
  qry.bind( 1, static_cast<long long>( this->pkgId ) );
  auto json = nlohmann::json::parse( ( *qry.begin() ).get<std::string>( 0 ) );
  /* We have to stash our `path' because `from_json' would clear it. */
  auto pathTmp = std::move( this->path );
  from_json( json, dynamic_cast<RawPackage &>( *this ) );
  /* Restore original `path'. */
  this->path = std::move( pathTmp );
}


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb


/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
