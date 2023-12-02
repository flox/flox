# Query Arguments -> SQL Query Pipeline

`pkgdb` exposes a number of interfaces for running complex queries against its
underlying collection of package databases, making them easy for callers to use.
These interfaces also help ensure consistent behavior and ranking between
_search_ and _resolution_ queries.

In order the ensure the behaviors of these two categories of queries are
consistent they share a common underlying _query builder_ called a
[PkgQuery](../include/flox/pkgdb/pkg-query.hh).
This class takes a set of filter rules provided by the caller, called
`PkgQueryArgs`, and can either emit a SQL `SELECT` statement as a string or
run that query on databases using `PkgQuery::bind( flox::pkgdb::database & )` or
`PkgQuery::execute( flox::pkgdb::database & )`.

Throughout this codebase you will find various `struct` and `class` definitions
which essentially exist to parse user input from JSON, TOML, or YAML and convert
them into `PkgQueryArgs` and `Registry<PkgDbInput>` _registries_; just remember
that past the tangled boilerplate of those routines - we're just moving fields
from the user's input, into these `PkgQueryArgs` or a set of `PkgDbInput`s.
This document will help you navigate these various structures and help you
identify common patterns among them.


## Terminology

- Registry : A collection of _flake ref inputs_ assigned _short names_
             or _aliases_ which comprise the pool of package sets to be queried.
             Not the same as Nix registry, but fucking close. We added
             additional fields (See [registry](./registry.md)).
- Descriptor : An abstract description of a dependency by indicating
               _requirements_ to be satisfied.
               These are simply put a collection of user defined filters.
               The most common descriptor is
               _"give me a package with the name X"_, or maybe 
               _"give me a package with the name X with version between 3-4"_.
- Preferences : Settings or filters which apply to _all_ descriptors or 
                registry inputs.
                These may overlap with some _descriptor_ filters, but may be
                defined _globally_ for convenience.
                These are things like
                _"limit search results to those with the following licenses: 
                X, Y, Z_, or
                _"treat prefer pre-release versions of sofware over the previous 
                major release"_.


## Query Argument and Registry Data

The pipeline from user input into `PkgQueryArgs` and `RegistryRaw` structures
is split across a few base classes in an attempt to avoid repeating common
processes in multiple places - this comes at the expense of making it slightly
harder for readers to trace; the aim of this section is to aggregate all of
classes related to this transformation in one place, and describe their
relationships to one another.

### flox::RegistryRaw

Declared in [<pkgdb>/include/flox/registry.hh](../include/flox/registry.hh).

This structure holds information about _flake inputs_ whose package databases
should be scraped, some settings associated with each to indicate which
_subtrees_ should be searched, and the _priority_ order results
should be _ranked_ by.

A more detailed look at _registries_ and their fields can be
found [here](./registry.md).


### flox::pkgdb::PkgQueryArgs

Declared in
[<pkgdb>/include/flox/pkgdb/pkg-query.hh](../include/flox/pkgdb/pkg-query.hh).

This is the _finalized_ set of arguments used to actually query a database.
It has the following fields which are translated into SQL query filters,
and in the case of the `semver` field additional filtering using `node-semver`
will be performed.

```c++
// NOTE: This document may be out of sync with `pkg-query.hh'.
//       The header itself is the _source of truth_.

struct PkgQueryArgs
{

   std::optional<std::string> name;    //< Filter results by exact `name`.
   std::optional<std::string> pname;   //< Filter results by exact `pname`.
   std::optional<std::string> version; //< Filter results by exact version.
   std::optional<std::string> semver;  //< Filter results by version range.
   
  /** Filter results by partial match on pname, attrName, or description. */
  std::optional<std::string> partialMatch;

  /** Filter results by partial match on pname or attrName. */
  std::optional<std::string> partialNameMatch;

  /** Filter results by an exact match on either `pname` or `attrName`. */
  std::optional<std::string> pnameOrAttrName;

  /** 
   * Filter results to those explicitly marked with the given licenses.
   *
   * NOTE: License strings should be SPDX Ids ( short names ).
   */
  std::optional<std::vector<std::string>> licenses;

  /** Whether to include packages which are explicitly marked `broken`. */
  bool allowBroken = false;

  /** Whether to include packages which are explicitly marked `unfree`. */
  bool allowUnfree = true;

  /** Whether pre-release versions should be ordered before releases. */
  bool preferPreReleases = false;

  /** 
   * Subtrees to search.
   * 
   * NOTE: `Subtree` is an enum of top level flake outputs, being one of
   * `"packages"` or `"legacyPackages"`.
   */
  std::optional<std::vector<Subtree>> subtrees;

  /** Systems to search. Defaults to the current system. */
  std::vector<std::string> systems = { nix::settings.thisSystem.get() };

  /**
   * Relative attribute path to package from its prefix.
   * For regular flakes it is the part following `system`.
   *
   * NOTE: @a flox::AttrPath is an alias of `std::vector<std::string>`.
   */
  std::optional<flox::AttrPath> relPath;
  
  // ...<SNIP>...
  
};
```


### flox::search::SearchQuery

Declared in
[<pkgdb>/include/flox/search/params.hh](../include/flox/search/params.hh).

This set of parameters is used by `pkgdb search` in order to support
`flox search` and `flox show` sub-commands.
It has the following declaration:

```c++
// NOTE: This document may be out of sync with `params.hh'.
//       The header itself is the _source of truth_.
/**
 * @brief A set of query parameters.
 *
 * This is essentially a reorganized form of @a flox::pkgdb::PkgQueryArgs
 * that is suited for JSON input.
 */
struct SearchQuery
{

  std::optional<std::string> name;
  std::optional<std::string> pname;
  std::optional<std::string> version;
  std::optional<std::string> semver;

  /** Filter results by partial match on pname, attrName, or description */
  std::optional<std::string> partialMatch;

  /** Filter results by partial match on pname or attrName. */
  std::optional<std::string> partialNameMatch;
  
  // ...<SNIP>...

};
```

Contained by:
- `flox::search::SearchParams`

For a detailed look at `SearchParams` see 
[pkgdb search](./search.md) documentation.


### flox::resolver::ManifestDescriptorRaw

Declared in
[<pkgdb>/include/flox/resolver/descriptor.hh](../include/flox/resolver/descriptor.hh).

This form of descriptor is found in `manifest.{toml,yaml,json}` files.

It currently uses an intermediate struct `ManifestDescriptor` as an intermediate
step in its conversion to `flox::pkgdb::PkgQueryArgs`; but these two structures
may be merged in the future.

It has the following declaration:
```c++
struct ManifestDescriptorRaw
{

public:

  /** 
   * Match `name`, `pname`, or `attrName`.
   * Maps to `flox::pkgdb::PkgQueryArgs::pnameOrAttrName`.
   */
  std::optional<std::string> name;

  /** 
   * Match `version` or `semver` if a modifier is present. 
   *
   * Strings beginning with an `=` will filter by exact match on `version`.
   * Any string which may be interpreted as a semantic version range will
   * filter on the `semver` field.
   * All other strings will filter by exact match on `version`.
   */
  std::optional<std::string> version;

  /** @brief A dot separated attribut path, or list representation. */
  using Path = std::variant<std::string, flox::AttrPath>;
  /** Match a relative path. */
  std::optional<Path> path;

  /**
   * @brief A dot separated attribut path, or list representation.
   *        May contain `null` members to represent _globs_.
   *
   * NOTE: `AttrPathGlob` is a `std::vector<std::optional<std::string>>`
   *       which represnts an absolute attribute path which may have
           `std::nullopt` as its second element to avoid indicating a
           particular system.
   */
  using AbsPath = std::variant<std::string, AttrPathGlob>;
  /** Match an absolute path, allowing globs for `system`. */
  std::optional<AbsPath> absPath;

  /** Only resolve for a given set of systems. */
  std::optional<std::vector<std::string>> systems;

  /** Whether resoution is allowed to fail without producing errors. */
  std::optional<bool> optional;

  /** Named _group_ that the package is a member of. */
  std::optional<std::string> packageGroup;

  /** Force resolution is a given input or _flake reference_. */
  std::optional<std::variant<std::string, nix::fetchers::Attrs>>
    packageRepository;

  /** Relative path to a `nix` expression file to be evaluated. */
  std::optional<std::string> input;

};
```

Contained by:
- `flox::resolver::ManifestRaw`


### flox::resolver::ManifestRaw

NOTE: Today this file is not used to support `flox search` but in the future
      it will.

Declared in
[<pkgdb>/include/flox/resolver/manifest.hh](../include/flox/resolver/manifest.hh).

This is our internal representation of a parsed `manifest.{toml,yaml,json}` file
which describes an environment.

The `flox::resolver::ManifestRaw` struct intends to exactly match the format
of the file so that it can be used to validate its syntax, and be converted
without any loss of detail into JSON ( to become a part of a lockfile ).
With that in mind, no default fields are handled in this struct; instead
handling of fallbacks is done in a related class
`flox::resolver::Manifest`.

While some fields in this file are not relevant to resolution/search all of them
will be shown here.
Here is its declaration:

```c++
/**
 * @brief A _raw_ description of an environment to be read from a file.
 *
 * This _raw_ struct is defined to generate parsers, and its declarations simply
 * represent what is considered _valid_.
 * On its own, it performs no real work, other than to validate the input.
 */
struct ManifestRaw
{

  // NOTE: Unrelated to queries
  struct EnvBase
  {
    std::optional<std::string> floxhub;
    std::optional<std::string> dir;
  };
  std::optional<EnvBase> envBase;

  struct Options
  {
    std::optional<std::vector<std::string>> systems;

    struct Allows
    {
      std::optional<bool>                     unfree;
      std::optional<bool>                     broken;
      std::optional<std::vector<std::string>> licenses;
    };
    std::optional<Allows> allow;

    struct Semver { std::optional<bool> preferPreReleases; };
    std::optional<Semver> semver;

    std::optional<std::string> packageGroupingStrategy;
    std::optional<std::string> activationStrategy;
    // TODO: Other options
  };
  Options options;

  // Contains descriptors to be resolved.
  // FIXME: This should be `ManifestDescriptorRaw`! ( to be fixed soon )
  std::unordered_map<std::string, ManifestDescriptor> install;

  RegistryRaw registry;

  // NOTE: Unrelated to queries
  std::unordered_map<std::string, std::string> vars;

  // NOTE: Unrelated to queries
  struct Hook
  {
    std::optional<std::string> script;
    std::optional<std::string> file;
  };
  std::optional<Hook> hook;

};
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
