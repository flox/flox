/* ========================================================================== *
 *
 * @file pkgdb/schemas.hh
 *
 * @brief SQL Schemas to initialize a package database.
 *
 *
 * -------------------------------------------------------------------------- */

#pragma once

/* -------------------------------------------------------------------------- */

namespace flox::pkgdb {

/* -------------------------------------------------------------------------- */

/* Holds metadata information about schema versions. */
static const char * sql_versions = R"SQL(
CREATE TABLE IF NOT EXISTS DbVersions (
  name     TEXT NOT NULL PRIMARY KEY
, version  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS DbScrapeMeta (
  key      TEXT NOT NULL PRIMARY KEY
, value    TEXT NOT NULL
)
)SQL";

/* -------------------------------------------------------------------------- */

static const char * sql_input = R"SQL(
CREATE TABLE IF NOT EXISTS LockedFlake (
  fingerprint  TEXT  PRIMARY KEY
, string       TEXT  NOT NULL
, attrs        JSON  NOT NULL
);

CREATE TRIGGER IF NOT EXISTS IT_LockedFlake AFTER INSERT ON LockedFlake
  WHEN ( 1 < ( SELECT COUNT( fingerprint ) FROM LockedFlake ) )
  BEGIN
    SELECT RAISE( ABORT, 'Cannot write conflicting LockedFlake info.' );
  END
)SQL";


/* -------------------------------------------------------------------------- */

static const char * sql_attrSets = R"SQL(
CREATE TABLE IF NOT EXISTS AttrSets (
  id        INTEGER       PRIMARY KEY
, parent    INTEGER
, attrName  VARCHAR( 255) NOT NULL
, done      BOOL          NOT NULL DEFAULT FALSE
, CONSTRAINT  UC_AttrSets UNIQUE ( parent, attrName )
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_AttrSets ON AttrSets ( parent, attrName );

CREATE TRIGGER IF NOT EXISTS IT_AttrSets AFTER INSERT ON AttrSets
  WHEN
    ( NEW.id = NEW.parent ) OR
    ( ( SELECT NEW.parent != 0 ) AND
      ( ( SELECT COUNT( id ) FROM AttrSets WHERE ( NEW.parent = AttrSets.id ) )
        < 1
      )
    )
  BEGIN
    SELECT RAISE( ABORT, 'No such AttrSets.id for parent.' );
  END
)SQL";


/* -------------------------------------------------------------------------- */

static const char * sql_packages = R"SQL(
CREATE TABLE IF NOT EXISTS Descriptions (
  id           INTEGER PRIMARY KEY
, description  TEXT    NOT NULL UNIQUE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_Descriptions
  ON Descriptions ( description );

CREATE TABLE IF NOT EXISTS Packages (
  id                INTEGER PRIMARY KEY
, parentId          INTEGER        NOT NULL
, attrName          VARCHAR( 255 ) NOT NULL
, name              VARCHAR( 255 ) NOT NULL
, pname             VARCHAR( 255 )
, version           VARCHAR( 127 )
, semver            VARCHAR( 127 )
, license           VARCHAR( 255 )
, outputs           JSON           NOT NULL
, outputsToInstall  JSON
, broken            BOOL
, unfree            BOOL
, descriptionId     INTEGER
, FOREIGN KEY ( parentId      ) REFERENCES AttrSets  ( id )
, FOREIGN KEY ( descriptionId ) REFERENCES Descriptions ( id     )
, CONSTRAINT UC_Packages UNIQUE ( parentId, attrName )
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_Packages
  ON Packages ( parentId, attrName )
)SQL";


/* -------------------------------------------------------------------------- */

static const char * sql_views = R"SQL(

-- A JSON list form of the _attribute path_ to an `AttrSets` row.
CREATE VIEW IF NOT EXISTS v_AttrPaths AS
  WITH Tree ( id, parent, attrName, subtree, system, path ) AS
  (
    SELECT id, parent, attrName
         , attrName                     AS subtree
         , NULL                         AS system
         , ( '["' || attrName || '"]' ) AS path
    FROM AttrSets WHERE ( parent = 0 )
    UNION ALL SELECT O.id, O.parent
                   , O.attrName
                   , Parent.subtree
                   , COALESCE( Parent.system, O.attrName ) AS system
                   , json_insert( Parent.path, '$[#]', O.attrName ) AS path
    FROM AttrSets O INNER JOIN Tree as Parent ON ( Parent.id = O.parent )
  ) SELECT * FROM Tree;


-- Splits semvers into their major, minor, patch, and pre-release tags.
CREATE VIEW IF NOT EXISTS v_Semvers AS SELECT
  semver
, major
, minor
, CASE WHEN ( length( mPatch ) < 1 ) THEN rest ELSE mPatch END AS patch
, CASE WHEN ( length( mPatch ) < 1 ) THEN NULL ELSE rest END   AS preTag
FROM (
  SELECT semver
       , major
       , minor
       , ( substr( rest, 0, instr( rest, '-' ) ) )  AS mPatch
       , ( substr( rest, instr( rest, '-' ) + 1 ) ) AS rest
  FROM (
    SELECT semver
         , major
         , ( substr( rest, 0, instr( rest, '.' ) ) )  AS minor
         , ( substr( rest, instr( rest, '.' ) + 1 ) ) AS rest
    FROM (
      SELECT semver
           , ( substr( semver, 0, instr( semver, '.' ) ) )  AS major
           , ( substr( semver, instr( semver, '.' ) + 1 ) ) AS rest
      FROM ( SELECT DISTINCT semver FROM Packages )
    )
  )
) ORDER BY major, minor, patch, preTag DESC NULLS FIRST;


-- Supplies additional version information identifying _date_ versions,
-- and categorizes versions into _types_.
CREATE VIEW IF NOT EXISTS v_PackagesVersions AS SELECT
  Packages.id
, CASE WHEN Packages.version IS NULL    THEN NULL
       WHEN Packages.semver IS NOT NULL THEN NULL
       WHEN ( SELECT Packages.version = date( Packages.version ) )
         THEN date( Packages.version )
       ELSE NULL
  END AS versionDate
, CASE WHEN Packages.version IS NULL                               THEN 3
       WHEN Packages.semver IS NOT NULL                            THEN 0
       WHEN ( SELECT Packages.version = date( Packages.version ) ) THEN 1
                                                                   ELSE 2
  END AS versionType
FROM Packages
LEFT OUTER JOIN v_Semvers ON ( Packages.semver = v_Semvers.semver );


-- Additional information about the _attribute path_ for a `Packages` row.
CREATE VIEW IF NOT EXISTS v_PackagesPaths AS SELECT
  Packages.id
, json_insert( v_AttrPaths.path, '$[#]', Packages.attrName ) AS path
, json_insert( json_remove( v_AttrPaths.path, '$[1]', '$[0]' )
             , '$[#]'
             , Packages.attrName
             ) AS relPath
, ( json_array_length( v_AttrPaths.path ) + 1 ) AS depth
, Packages.attrName AS attrName
FROM Packages INNER JOIN v_AttrPaths ON ( Packages.parentId = v_AttrPaths.id );


-- Aggregates columns used for searching packages.
CREATE VIEW IF NOT EXISTS v_PackagesSearch AS SELECT
  Packages.id
, v_AttrPaths.subtree
, v_AttrPaths.system
, v_PackagesPaths.path
, v_PackagesPaths.relPath
, v_PackagesPaths.depth
, Packages.name
, Packages.attrName
, Packages.pname
, v_PackagesPaths.attrName
, Packages.version
, v_PackagesVersions.versionDate
, Packages.semver
, v_Semvers.major
, v_Semvers.minor
, v_Semvers.patch
, v_Semvers.preTag
, v_PackagesVersions.versionType
, Packages.license
, Packages.broken
, CASE WHEN broken IS NULL THEN 1
       WHEN broken         THEN 2
                           ELSE 0
  END AS brokenRank
, Packages.unfree
, CASE WHEN unfree IS NULL THEN 1
       WHEN unfree         THEN 2
                           ELSE 0
  END AS unfreeRank
, Descriptions.description
FROM Packages
LEFT OUTER JOIN Descriptions ON ( Packages.descriptionId = Descriptions.id )
LEFT OUTER JOIN v_Semvers    ON ( Packages.semver = v_Semvers.semver )
     INNER JOIN v_AttrPaths        ON ( Packages.parentId = v_AttrPaths.id )
     INNER JOIN v_PackagesPaths    ON ( Packages.id = v_PackagesPaths.id )
     INNER JOIN v_PackagesVersions ON ( Packages.id = v_PackagesVersions.id )
)SQL";


/* -------------------------------------------------------------------------- */

}  // namespace flox::pkgdb

/* -------------------------------------------------------------------------- *
 *
 *
 *
 * ========================================================================== */
