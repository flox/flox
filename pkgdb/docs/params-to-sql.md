# Query Arguments -> SQL Query Pipeline

## Overview
`pkgdb` provides interfaces for running complex queries against its underlying
collection of package databases, making them easy for callers to use.
These interfaces also help ensure consistent behavior and ranking between
_search_ and _resolution_ queries.

These two categories of queries share a common underlying _query builder_ called
a [PkgQuery](../include/flox/pkgdb/pkg-query.hh).
The caller provides filters to the query builder in the form of `PkgQueryArgs`.
`PkgQuery` can either emit a SQL `SELECT` statement as a string or run that
query on databases using `PkgQuery::bind( flox::pkgdb::database & )` or
`PkgQuery::execute( flox::pkgdb::database & )`.
The difference between `bind` and `execute` will be explained later.

Throughout this codebase you will find various `struct` and `class` definitions
which essentially exist to hold the parsed user input and convert it into
`PkgQueryArgs` and `Registry<PkgDbInput>` _registries_.
This document will help you navigate these structures and the flow of data in 
a query.


## Terminology

- **Registry**: A collection of flake ref inputs with aliases.
  - Defines the pool of package sets to be queried.
  - Not the same as a Nix registry, but essentially a superset
    ( we added fields ).
  - See the [registry schema](./registry.md#schema).
- **Descriptor** : An abstract description of a package, including various
  requirements to be satisfied.
  - The requirements put constraints on the name, version, origin, etc.
  - See [Descriptor](./manifests.md#manifest).
- **Preferences** : Settings or filters which may apply to all descriptors or
  registry inputs.
  - These may overlap with some descriptor fields, but may be defined globally
    for convenience.
  - Global filters include things like disallowing non-free licenses, etc.

## Search query data flow

### `flox` constructs a `SearchParams` and calls `pkgdb`
A search query from `flox` fills out a `SearchParams` struct
( in the Rust codebase ), which is serialized to JSON, leaving out any fields
which were not provided (most fields are optional).
Fields that aren't provided by `flox` are assumed to be handled with default
values by `pkgdb`.
The `SearchParams` struct in `flox` is meant to model the
`flox::search::SearchParams` struct in `pkgdb` with the `Query` struct in `flox`
mapping to the `flox::search::SearchQuery` struct in `pkgdb`.

### `pkgdb` parses the provided search parameters

The `SearchCommand::addSearchParamsArgs` function creates a positional argument
to `pkgdb search` that when parsed fills out a `flox::search::SearchParams` and
stores it on the `SearchCommand` struct.

`flox::search::SearchParams` contains these components:
- A `flox::resolver::GlobalManifestRaw`, a path to a global manifest file,
  or nothing.
- A `flox::resolver::ManifestRaw`, a path to a manifest file, or nothing.
- A `flox::resolver::LockfileRaw`, a path to a lockfile, or nothing.
- A `flox::search::SearchQuery`.

### An abstract environment is created

`pkgdb` performs a search in the context of an environment, where the context is
comprised of a global manifest, the environment's manifest, and a lockfile if
one exists.
The presence of the lockfile enables searches to be done against inputs that
have already been locked.
I say that this environment is abstract to contrast with how `flox` and `pkgdb` 
actually build environments that the user can activate.

`SearchCommand` inherits from the `GAEnvironmentMixin` base class, which defines
the functionality for initializing this abstract environment.
The environment is initialized in `SearchCommand::run` via the call
to `GAEnvironmentMixin::initEnvironment`.
For each of the global manifest, manifest, and lockfile, the corresponding
`*Raw` struct is initialized in the following steps:
- If a path was provided, it's stored on the corresponding field from the
  `GAEnvironmentMixin` base class.
- The file at the provided path is read and converted to JSON (manifests may be
  written as TOML, JSON, or YAML at the moment).
- If no path was provided, we attempt to grab the inline JSON from the query
  parameters.
- If we have a JSON blob (either from a file or from the query parameters) we
  attempt to construct the corresponding `*Raw` struct via the `from_json`
  functions that are defined for each of the structs.
- Once the raw structs are filled out, the non-raw structs are constructed from
  the raw input, validating the input in the process.

The `*Raw` structs are meant to faithfully represent the raw input provided by a
consumer of `pkgdb`, meaning they are intentionally permissive in the data that
they're allowed to contain (e.g. the code will compile even if two conflicting
options are provided).

Finally, a `flox::resolver::Environment` (not the same thing as a
`GAEnvironmentMixin`) is constructed from the global manifest, manifest, and
lockfile if they exist.

### Options are merged
As mentioned above, the `PkgQueryArgs` struct holds filters for the query.
A base `PkgQueryArgs` is assembled by merging the options found in the global
manifest, manifest, and lockfile.
Note that the lockfile contains a record of the manifest that was locked to
create it, so it therefore contains a set of options itself.
After this the options from the `SearchQuery` are applied to construct the 
final `PkgQueryArgs`.

Thus, the order of precedence for options ( lowest priority first ) is:
- global manifest
- lockfile's manifest record
- manifest
- query parameters

At this point we're ready to construct the database query.

### The database query is constructed

The query is constructed when the `PkgQuery` is initialized from
the `PkgQueryArgs`.
`PkgQuery` stores an internal list of `SELECT`, `WHERE`, etc clauses that get
built up as the query arguments are processed.
The details of query building are covered in a separate section.

### The query is executed

`PkgQuery` has two main methods for executing queries, `execute` and `bind`,
with `bind` being the lower level of the two.
For searches the query is executed via `PkgQuery::execute` once for each
database i.e. once for each registry input.
Note that this means you can't simply add a `LIMIT N` clause to the query
because you would get `N` results _from each database_.

Calling `PkgQuery::bind` turns the internal clauses into one query, binds any
query variables, and produces a `sqlite3pp::query`.
The `sqlite3pp::query` produces an iterator of exported columns, where the
columns to export are the row-id and the `semver` version by default.
Exported columns can be set through one of the `PkgQuery` constructors.
At this point you have an iterator of rows that match the query 
_aside from any semver considerations_.
Semantic version filtering is handled by `PkgQuery::execute`.

Calling `PkgQuery::execute` internally calls `PkgQuery::bind` and then performs
semver filtering if necessary, returning a `std::vector` of the row-ids that
satisfy the semver requirement.
Once the list of row-ids is constructed, the database is queried for each row
and JSON output is constructed for each result.
Search results are printed to `stdout` in JSONLines format
( one JSON object per line ).

## Table structure

### Main table schemas
Table schemas are stored in `src/pkgdb/schemas.hh`.
The main entities are `AttrSets` and `Packages`.
`AttrSets` form a tree structure via `AttrSet.parent` references and `Packages`
refer to their containing attribute set via `Packages.parentId`.
As an example, the installable `packages.aarch64-darwin.foo` would be
represented like so:
- a `Package` where `Package.attrName = 'foo'`
- an `AttrSets` where `AttrSets.attrName = 'aarch64-darwin'`
- an `AttrSets` where `AttrSets.attrName = 'packages'`
- `foo.parentId = aarch64-darwin.id`
- `aarch64-darwin.parent = packages.id`
- `packages.id = NULL`

An `AttrSets.parent` where `parent = NULL` indicates that the attrset is a root
of a tree.
There is also an `AttrSets.done` field which indicates that an `AttrSets` and
all of its children have been fully scraped i.e. it is a progress indicator
during scraping and doesn't have any meaning after the database has
been constructed.

The `Packages` table defines the data that is known about a particular package.
If a package explicitly defines a `pname` and `version` they will be used
directly, otherwise they will be parsed from `name`.
If a `version` can be interpreted as a semantic version it will be stored
as such.

There is also a table for `Descriptions` so that descriptions can be
deduplicated across different systems.

### Views
Many of the query fields are computed rather than being stored directly in
the database.
For instance, the `v_AttrPaths` view is a recursive query that collects the
entire attribute path for a single `Packages`.
The `v_PackagesSearch` view collects just about all the information you could
want to search about a package into a single place that can be keyed into via
a `Packages.id`.
The final query is done against this `v_PackageSearch` view and the columns
exported from the search query are chosen from the columns of `v_PackageSearch`.

## Query building
Query building happens in `src/pkgdb/pkg-query.cc`, starting
with `PkgQuery::init`.
The final query is converted to a string in `PkgQuery::str`.

While reading the query building functions it's important to remember that
`PkgQuery: PkgQueryArgs` so it inherits all of the fields from the query.
Another thing to point out is that the parameters names in the JSON query are
not the same as those in `PkgQueryArgs` or `SearchQuery`.
You can see the mapping of names in `src/search/params.cc` in the
`from_json( const nlohmann::json & jfrom, SearchQuery & qry )` function.
Most notably, the `match` JSON parameter becomes `partialMatch` and `match-name` 
becomes `partialNameMatch`.

`PkgQuery::initMatch` handles creating boolean fields that represent whether the
`match` or `match-name` query parameters are fuzzy matches for the `pname`,
`name`, or `attrName` of a package.
Strict matching against `name`, `pname`, `version`, `licenses`, `broken`,
`unfree`, and `relPath` is done in `PkgQuery::init`.
The subtrees and systems are filtered in `PkgQuery::initSubtrees` and
`PkgQuery::initSystems` respectively.

The actual order of search results is determined by the `ORDER BY` clause
generated in `PkgQuery::initOrderBy`.
This ranks search results based on how the query provides the attribute name
(`pname`, `name`, `match`, `match-name`, or `path`), whether the match is exact
vs. partial, the depth of the attribute (is it `foo` or `some.packageset.foo`),
and whether it matches the description instead of the package name.
Incorrect search results can probably be attributed to tweaks required to the
ordering of the fields that appear in this `ORDER BY` clause.

### Example query
For the search query
```
$ pkgdb search --ga-registry --match-name hello --dump-query
```
performed on an `aarch64-darwin` system the generated query is
```sql
SELECT
    id,
    semver
FROM
    (
        SELECT
            *,
            NULL AS exactPname,
            NULL AS exactAttrName,
            (
                ('%' || LOWER(pname) || '%') = LOWER(:partialMatch)
            ) AS matchExactPname,
            (
                ('%' || LOWER(attrName) || '%') = LOWER(:partialMatch)
            ) AS matchExactAttrName,
            (pname LIKE :partialMatch) AS matchPartialPname,
            (attrName LIKE :partialMatch) AS matchPartialAttrName,
            NULL AS matchPartialDescription,
            0 AS subtreesRank,
            0 AS systemsRank
        FROM
            v_PackagesSearch
        WHERE
            (
                (
                    matchExactPname
                    OR matchExactAttrName
                    OR matchPartialPname
                    OR matchPartialAttrName
                )
            )
            AND (
                (broken IS NULL)
                OR (broken = FALSE)
            )
            AND (system IN ('aarch64-darwin'))
        ORDER BY
            exactPname DESC,
            matchExactPname DESC,
            exactAttrName DESC,
            matchExactAttrName DESC,
            depth ASC,
            matchPartialPname DESC,
            matchPartialAttrName DESC,
            matchPartialDescription DESC,
            subtreesRank ASC,
            systemsRank ASC,
            pname ASC,
            versionType ASC,
            preTag DESC NULLS FIRST,
            major DESC NULLS LAST,
            minor DESC NULLS LAST,
            patch DESC NULLS LAST,
            versionDate DESC NULLS LAST -- Lexicographic as fallback for
                                        -- misc. versions
,
            v_PackagesSearch.version ASC NULLS LAST,
            brokenRank ASC,
            unfreeRank ASC,
            attrName ASC
    )
```

## Common Routines

### `getBaseQueryArgs`

This routine provides a _base_ set of `PkgQueryArgs` based on global settings
so that they may be used to create individual descriptors' queries.

This is currently only in use by `flox::resolver::Manifest`, but is
preferred for any parameter set containing _global_ settings.


### `fillPkgQueryArgs`

This routine appears on most parameter sets and is used to move parameter fields
into a `PkgQueryArgs` struct.

In some cases these are implemented such that they will not override existing
values or in other cases they will handle various fallbacks if a value is unset.


### `to_json` and `from_json`

These routines allow a struct to be converted to/from JSON.
These may be derived from templates, generated using `NLOHMANN_DEFINE_TYPE_*`
macros, or explicitly defined.

Our preference is to provide explicit definitions for these routines in order to
emit customized `flox::FloxException` messages instead of the default messages
created by `nlohmann::json::*` routines.

In some cases `to_json` may not be required or implemented, but nearly all
parameters have an implementation of `from_json`.


### `clear`

Clears a set of parameters to their default state.
This is largely used to recycle an existing parameter object
without reallocating.


### `getRegistryRaw`

This routine appears on parameter sets which contain a `flox::RegistryRaw`
member or have the ability to reinterpret some of their fields to create
a `flox::RegistryRaw`.

It largely exists for convenience and to enforce privacy on member variables;
however the constructor for `flox::pkgdb::PkgDbRegistryMixin`, which is
responsible for instantiating `flox::pkgdb::PkgDbInput` elements from a
`flox::RegistryRaw` requires child classes to implement this interface.


### `getSystems`

This routine appears on parameter sets which contain a `systems` list
member or have the ability to reinterpret some of their fields to create
a list of _target_ `systems`.

It largely exists for convenience and to enforce privacy on member variables;
however the constructor for `flox::pkgdb::PkgDbRegistryMixin`, which is
responsible for instantiating `flox::pkgdb::PkgDbInput` elements from a
`flox::RegistryRaw` requires child classes to implement this interface.
